# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend for the EmuChef config editor
protocol. It is runnable independently and is the Rust sidecar runtime used by
the Tauri editor. Phase 6V adds host-target Tauri v2 `externalBin` packaging for
this process. Phase 6W retires Python from the Tauri runtime path but keeps
Python CLI, PySide6, and golden-generation code temporarily as
legacy/reference/developer tooling. Broad CLI/planner/executor replacement and
full Python deletion remain separate later work.

Through Phase 6U it implements only:

- `hello`
- `listStepSpecs`
- `emitRecipeYamlFromPath`
- `validateRecipePath`
- sidecar-only `openRecipe`
- sidecar-only `getDocument`
- sidecar-only `saveRecipe`
- sidecar-only `saveRecipeAs`
- sidecar-only `closeDocument`
- sidecar-only `applyRecipeCommand`
- sidecar-only `undo`
- sidecar-only `redo`
- sidecar-only `emitYaml`
- sidecar-only `validate`
- sidecar-only `getRefIndex`
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling
- authored recipe YAML load/emit skeletons for focused migration fixtures
- editor-local authored recipe validation diagnostics for focused Phase 6K
  fixtures
- authoredRoot/catalog-context recipe validation diagnostics for focused Phase
  6L fixtures
- in-memory sidecar document sessions backed by the authored recipe model
- snapshot undo/redo for open document sessions
- the Python-compatible `SetOverviewField` command for recipe `name` and
  `description`
- Python-compatible non-step `applyRecipeCommand` mutations for the currently
  modeled input, artifact, and artifact group command families
- Python-compatible step lifecycle and dependency `applyRecipeCommand`
  mutations for the currently modeled step fields
- Python-compatible step params and advanced internals `applyRecipeCommand`
  mutations for the currently modeled step fields
- generated `RecipeDocumentDto.refIndex` data for the currently modeled
  authored recipe features
- an internal-only, fixture-scoped declarative planner that emits a
  Python-shaped `PlanningResult`/`ExecutionPlan` for focused Phase 6M and 6N
  tests
- a dev-only Rust planner shadow binary that emits the private Rust
  `PlanningResult` as pretty JSON for explicit authored-root/device-plan inputs
- a crate-local P8V/P8W/P8X ADB `getprop` command model, supplied-output
  parser, live adapter foundation, and explicit dev-only shadow-binary probe
  mode for future Rust planner probing ownership
- an internal-only, fixture-scoped executor skeleton that emits Python-shaped
  `ExecutionRunResult` values for selected safe dry-run Phase 6O tests
- temp-dir-confined filesystem/artifact executor behavior for selected Phase 6P
  fixtures
- selected fake-device/DryRunAdb executor behavior for Phase 6Q
  device/app/permission fixtures
- internal real-ADB adapter foundations and ignored/manual Phase 6R tests for
  selected device/app/permission behavior
- a minimal crate-local Phase 6S Rust CLI skeleton for selected Python CLI
  parity fixtures

It reports only capabilities that are implemented in this crate:

```json
{
  "protocolVersion": 1,
  "capabilities": [
    "listStepSpecs",
    "emitRecipeYamlFromPath",
    "validateRecipePath",
    "openRecipe",
    "getDocument",
    "saveRecipe",
    "saveRecipeAs",
    "closeDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "emitYaml",
    "validate",
    "getRefIndex"
  ]
}
```

Reporting document-session capabilities means the Rust backend supports those
requests in JSONL sidecar mode only. One-shot mode remains stateless and does
not expose persistent document session APIs.

Reporting these capabilities does not mean the Rust backend has full product
parity with Python. Phase 6U hard-wires the Tauri editor runtime to launch this
Rust sidecar in local/dev flows with no backend selector, toggle, environment
variable, config option, UI switch, or Python fallback. Phase 6V keeps that
hard-cutover policy and packages the same Rust process as a Tauri v2
`externalBin`. The app-local Tauri hooks build and copy debug or release
sidecar artifacts before normal Tauri dev/build commands.

The Python backend, Python CLI, and PySide6 editor remain in the repository only
as legacy/reference/developer/golden tooling. They are not Tauri editor runtime
backends or packaged-editor fallbacks. Python deletion and broad
CLI/planner/executor replacement remain later confirmed cutover work.

## Tauri Sidecar Packaging

The Tauri config editor packages this crate as a separate sidecar process with
Tauri v2 `externalBin`; it does not link the backend as a library. App-local
commands live in `apps/config-editor`:

```bash
npm run sidecar:dev
npm run sidecar:build
npm run tauri build
```

`sidecar:dev` builds the debug Cargo binary and prepares the Tauri externalBin
input for development. `sidecar:build` builds the release Cargo binary and
prepares `src-tauri/binaries/emuchef-rust-backend-$TARGET_TRIPLE`, adding
`.exe` for Windows triples. The script verifies `rustc --print host-tuple` and
only supports host-target preparation in Phase 6V; cross-compilation and release
CI are deferred.

Tauri strips the target triple when bundling. Packaged apps launch
`emuchef-rust-backend --sidecar` from the app executable directory, while
development and tests continue to resolve Cargo `target/debug` binaries.

This package does not implement full planner behavior, full executor behavior,
Python bundling, or Python deletion. Phase 6K replaced the earlier basic
validation skeleton with fixture-covered editor-local validation parity for the
current Rust recipe model scope. Phase 6L adds fixture-covered
authoredRoot/catalog-context validation for Python-verified recipe dependency
diagnostics. Phase 6M adds a minimal internal declarative planner skeleton for
focused fixtures only. Phase 6N expands that private planner to broader
Python-golden-backed parity for current built-in planning mappings, dependency
expansion, input binding/default/multiple values, artifact/group planning,
StepSpec defaults, refs, `skip_if`, `verify`, and planner result status/error
shape for selected fixtures. Phase 6O adds a private safe executor skeleton for
selected Python dry-run result fixtures. Phase 6P expands that private executor
with selected filesystem/artifact behavior, but every filesystem mutation is
confined to explicit test-owned temp roots. Phase 6Q adds selected fake-device
ADB parity. Phase 6R adds an internal real-ADB adapter foundation and ignored
manual tests, but normal tests still do not perform real device checks, real
network downloads, permission grants, ADB/device operations, app lifecycle
operations, or install operations. Python remains the legacy/reference source
for broader CLI/planner/executor behavior until parity or retirement is
confirmed.

## Phase 6S CLI Scope

Phase 6S adds a minimal Rust CLI skeleton inside this standalone crate. It is a
crate-local experimental parity surface, not the user-facing replacement for the
Python `emuchef` CLI. The Python CLI entrypoint in `pyproject.toml` remains
unchanged temporarily for legacy/reference/developer/golden workflows, and the
Rust CLI subset is not packaged as a replacement for the Python CLI. There is no
backend selector, backend toggle, environment variable, config option, UI switch,
or Python fallback for the Tauri editor runtime.

The Python CLI inventory verified from `src/emuchef/cli.py` is:

| Python command | Python options verified for Phase 6S | Phase 6S Rust status |
| --- | --- | --- |
| `draft` | `--authored-root`, `--device-plan`, `--ops`, `--bind`, device facts, common flags | Deferred; Python resolves ADB device facts. |
| `plan` | `--authored-root`, `--device-plan`, `--ops`, `--bind`, `--output`, device facts, common flags | Deferred; Python resolves ADB device facts before planning. |
| `detect` | `--serial`, common flags | Deferred; real ADB/device behavior. |
| `detect-profiles` | `--authored-root`, `--serial`, common flags | Deferred; real ADB/device behavior. |
| `validate` | optional `path`, `--authored-root`, common flags | Implemented only for explicit recipe-file paths with optional `--authored-root`; default/catalog validation and verbose/debug/ADB flags are deferred. |
| `apply` | required `--plan-file`, optional `--serial`, `--dry-run`, common flags | Implemented only for non-verbose `--dry-run` selected fixtures; ADB, verbose/debug, inputs, artifacts, and real execution are deferred. |

Python does not expose an `execute` command. Phase 6S therefore implements the
Python-backed `apply --plan-file <path> --dry-run` spelling and does not invent
`execute --dry-run`.

The Python plan-file loader verified from
`src/emuchef/io/execution_plan_io.py` accepts YAML loaded with `safe_load`.
Supported Phase 6S Rust fixtures are the same Python-supported representations:

- `kind: execution_plan`
- `kind: planning_result` with an `execution_plan` mapping

Authored recipe paths are not execution plan files. Rust Phase 6S does not parse
authored recipes as `apply` input, does not run real execution, and refuses
non-dry-run `apply` execution. It also rejects plan files with top-level
`inputs` or `artifacts` instead of silently diverging from Python's broader
plan-file support; those broader dry-run fixtures remain deferred.

### Phase 6S Output Parity

The Rust CLI tests use checked-in, source-backed expectations from the Python
CLI. Normal `cargo test` does not invoke Python. The selected parity targets are:

- `validate` text summaries for explicit recipe files: `Validation status: ...`,
  `Validated paths:`, grouped `Issues:`, issue codes/messages, field lines,
  stdout/stderr split, and exit status.
- `apply --dry-run` progress and summary text: checking/executing/verifying/
  finished lines, `Dry run: success|failed`, count labels, permission summary
  labels, stderr failure markers, and exit status.
- legacy Rust binary dispatch: JSON one-shot and `--sidecar` behavior remain
  unchanged. Single unknown non-JSON arguments such as `foo` and malformed JSON
  such as `{bad` still use one-shot malformed JSON behavior. The recognized
  single command `validate` is deliberately classified as the CLI command and
  reports the Phase 6S explicit-path requirement instead of emitting an API
  envelope.

Byte-for-byte parity is limited to selected fixture output. Broader Python CLI
behavior remains the reference and is deferred rather than approximated.

### Phase 6S Safety

Normal Phase 6S tests do not require Python, ADB, Android devices, APKs, network
access, package metadata changes, install scripts, root Cargo workspace changes,
Tauri Cargo changes, or editor/frontend changes. Dry-run execution uses the
existing fake dry-run executor path and test-owned temp files. Real ADB remains
manual/internal from Phase 6R only.

The Python CLI reference commands used to verify selected Phase 6S output can be
run from the repository root with the documented dependency pattern:

```bash
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef validate --authored-root authored
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef validate crates/emuchef-rust-backend/tests/fixtures/recipes/invalid_top_level_permissions.yaml
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef apply --plan-file <temp-plan.yaml> --dry-run
```

## Phase 6N Planner Scope

Phase 6M added `src/planner.rs`, an internal Rust module used only by
crate-local tests. Phase 6N expands that same private module; it is still not a
public crate API, protocol request, CLI command, Tauri command, TypeScript API,
backend selector, or config/env toggle. It emits the Python planner's serialized
`PlanningResult` shape with a nested `ExecutionPlan` for focused fixtures. The
parity target is Python's `Planner.start_session(...).emit_execution_plan()`
path, not CLI summary text or executor behavior.

The planner input is explicit fixture data: loaded Rust `Recipe` values, selected
recipe refs, a supplied device-plan ref, a supplied device-profile ref, a supplied
`DeviceContext`, supplied `RuntimeCapabilities`, and optional input bindings.
`DeviceContext` is modeled only to serialize the Python `ExecutionPlan` golden
shape. `RuntimeCapabilities` values in tests mirror the Python fixture defaults
from `tests/support.py`; Rust does not resolve profiles, probe devices, or invent
runtime capability defaults.

Planner fixture loading reads only top-level `authored/recipes/*.yml` /
`authored/recipes/*.yaml` files from the Phase 6L-style authoredRoot fixture
trees. It does not scan apps, profiles, device plans, nested recipe directories,
templates, project roots, or runtime filesystem state. Dependency expansion uses
only the loaded recipe metadata and preserves Python's dependency-before-selected
recipe order for the covered fixtures.

Phase 6N planner output parity is functional/semantic. Byte-for-byte JSON output
is not a guarantee, but the tests compare structured `PlanningResult` fields,
execution step order, ids, dependencies, refs, materialized StepSpec defaults,
artifact expansion, input binding/default values, optional-input pruning,
warnings/errors/status, and no-side-effect behavior. The Phase 6M and 6N JSON
fixtures under `tests/fixtures/python_goldens/phase6m_*` and
`tests/fixtures/python_goldens/phase6n_*` are generated from the actual Python
planner API. Phase 6N normalizes absolute repo-root paths in new goldens to
`$REPO_ROOT/...`.

Phase 6N does not execute steps. It does not create output directories, mutate
authored YAML, mutate document-session state, copy files, download artifacts,
extract archives, inspect devices, call ADB, grant permissions, run subprocesses,
perform network checks, add production packaging, bundle Python, or replace the
Python backend. Dependency cycles and missing recipe dependencies are still
covered at the authoredRoot/catalog validation boundary when Python catches them
before planning.

### Built-In Planner Behavior

| Step type | Phase 6N status | Notes |
| --- | --- | --- |
| `wait` | Implemented for fixture coverage | Emits literal params, participates in selected-step and dependency ordering tests, and does not sleep or run subprocesses. |
| `resolve_artifacts` | Implemented for fixture coverage | Expands `artifacts` and `artifact_groups` into execution artifact ids. Does not download, resolve URLs, or inspect caches. |
| `extract_artifacts` | Implemented for fixture coverage | Expands artifact selections and materializes StepSpec default `extract_on: host`, with explicit overrides preserved. Does not extract archives or touch host/device files. |
| `extract_archive` | Implemented for fixture coverage | Normalizes top-level artifact refs and materializes StepSpec default `cleanup: true`. Does not extract archives. |
| `copy_files` | Implemented for fixture coverage | Normalizes top-level refs, supports shorthand step-output refs, materializes StepSpec default `copy_policy: merge`, preserves explicit overrides, and emits declarative params only. Does not copy files. |
| `install_apk` | Implemented for fixture coverage | Normalizes the `app` artifact ref and materializes StepSpec default `replace_existing: false`. Does not install APKs. |
| `grant_permissions` | Implemented for fixture coverage | Keeps runtime/appops/policy params step-local and does not emit a separate permission plan or grant permissions. |
| `launch_app` | Implemented for fixture coverage | Emits literal package/activity params only. Does not launch apps. |
| `force_stop_app` | Implemented for fixture coverage | Emits literal package params and participates in capability pruning tests. Does not inspect or stop apps. |

Deferred planner gaps remain intentionally narrow: broad template expansion,
full Python draft conflict resolution for mutually selected `conflicts_with`
steps, executor/runtime `skip_if` and `verify` evaluation, device/profile
inference, network or cache inspection, filesystem existence checks, and
side-effecting artifact/app/permission operations are outside Phase 6N. The
Phase 6N fixtures cover `conflicts_with` serialization only when the conflicting
step is unavailable through fixture capabilities.

## Dev-Only Planner Shadow Command

The crate includes a developer-only `emuchef-plan-shadow` binary for manual Rust
planner migration inspection:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

The command loads checked-in authored recipe, device-plan, and device-profile
inputs through `PlannerInput::from_authored_device_plan(...)`, calls the private
Rust `plan_execution(...)` path, and writes pretty JSON `PlanningResult` output
to stdout. Planner success exits `0`; planner error results exit non-zero while
still writing the structured result to stdout. Argument/usage errors and
authored-root/device-plan load failures are process errors: they write stable
stderr text and no stdout JSON.

P8J adds explicit device context inputs to this dev-only command:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --manufacturer AYANEO \
  --model "Pocket S Mini" \
  --android-version 13 \
  --device-tag handheld
```

Explicit `--manufacturer`, `--model`, and `--android-version` values override
the synthetic/profile-derived planner context for that invocation. Repeated
`--device-tag` values replace profile-derived tags exactly in the supplied
order. When no explicit tags are supplied, profile-derived tags remain
unchanged. Without an explicit detected-facts source, the shadow command does
not probe devices, invoke ADB, create detected-device facts, or emit
detected-device profile mismatch warnings.

P8R adds a local detected-facts fixture harness to this dev-only command:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --detected-facts-json /path/to/detected-facts.json
```

The fixture file is strict JSON matching `DetectedDeviceFacts`. Scalar fields
are optional, omitted `device_tags` defaults to an empty list, and unknown fields
are rejected. Fixture mode runs the fake/test-backed detected-facts
planning-result composition path, preserves normal shadow `PlanningResult` JSON
output, and does not probe devices or invoke ADB. Effective context precedence is
synthetic/profile context -> detected fixture facts -> explicit CLI context
overrides. When explicit context flags and a fixture are both supplied, the
emitted `execution_plan.device_context` reflects the explicit overrides, while
`device_profile_mismatch` warnings still evaluate the fixture facts.
`--detected-facts-json <path>` is mutually exclusive with live ADB probing.
`emuchef plan --planner-backend rust-experimental` can explicitly forward a
local fixture with the Python-facing `--rust-detected-facts-json <path>` flag.
Python forwards the exact string as `--detected-facts-json <path>` to this
shadow binary and does not open, stat, expand, normalize, parse, or
schema-validate the fixture. The raw Rust `--detected-facts-json` flag remains
unrecognized by Python CLI routes, and `rust-shadow` does not accept the Python
wrapper flag.

P8X adds explicit live ADB getprop probing only to the direct dev-only shadow
binary:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --probe-adb-getprop \
  --adb-path adb \
  --serial SERIAL123
```

`--adb-path` defaults to `adb` when `--probe-adb-getprop` is supplied, and
`--serial` is optional. `--adb-path` and `--serial` are usage errors without
`--probe-adb-getprop` so ADB configuration cannot silently become a no-op. Live
mode builds `AdbProbeConfig`, executes `AdbDeviceProbe<ProcessCommandRunner>`,
parses `getprop` stdout into `DetectedDeviceFacts`, and routes those facts
through the same detected-facts planning-result composition path as fixture
mode. Probe launch failures and non-zero ADB exits are process errors with
stable `adb_probe_unavailable` or `adb_probe_failed` stderr classifications and
no stdout JSON.

P8Y adds optional/manual smoke evidence for that direct live mode:

```bash
python3 tools/smoke_rust_shadow_live_adb_probe.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --adb-path adb \
  --serial <selected-device-serial>
```

The smoke invokes only the supplied shadow binary with `--probe-adb-getprop`,
requires explicit `--adb-path` and `--serial`, and emits deterministic JSON
with scrubbed command metadata. It does not discover devices, run `adb devices`,
invoke Cargo, call Python CLI routes, reuse fixture or matrix smoke tooling,
write artifacts, run executor/apply behavior, touch Tauri/protocol behavior, or
participate in normal runtime checks or the static readiness gate. A
`device_profile_mismatch` warning is acceptable smoke evidence when the selected
device intentionally does not match the authored plan. Production route-level
probing, production mismatch-warning parity, and Python planner deletion remain
future work.

P8Z adds explicit Python CLI forwarding for live probe intent only through
`emuchef plan --planner-backend rust-experimental`:

```bash
python3 -m emuchef plan \
  --planner-backend rust-experimental \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --rust-probe-adb-getprop \
  --rust-adb-path adb \
  --rust-serial <selected-device-serial>
```

Python forwards those wrapper flags to the supplied shadow binary as
`--probe-adb-getprop`, `--adb-path <path>`, and `--serial <serial>`, then keeps
using Python-compatible output formatting for usable Rust `PlanningResult` JSON.
Python does not invoke ADB, discover devices, run `adb devices`, parse
`getprop`, or validate, normalize, expand, or stat the forwarded ADB path or
serial. The default Python backend and `rust-shadow` reject the wrapper flags
before ADB resolution, planner/session construction, device probing, or
subprocess execution. `--rust-detected-facts-json <path>` fixture forwarding and
live-probe forwarding are mutually exclusive detected-facts sources. P8Z is not
default planner cutover, not production route-level probing parity, not
readiness-gate executed evidence, not a smoke-runner change, and not Python
planner deletion.

P8AA adds optional/manual smoke evidence for that Python live-probe forwarding
route:

```bash
python3 tools/smoke_rust_experimental_live_adb_probe.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --adb-path adb \
  --serial <selected-device-serial>
```

The smoke invokes `python3 -m emuchef plan --planner-backend
rust-experimental` with the Python wrapper live-probe flags. It does not call
the Rust shadow binary directly, discover devices, run `adb devices`, invoke
Cargo, or inspect, normalize, expand, or stat the supplied ADB path or serial.
Successful route cases must emit Python-compatible output; raw Rust JSON stdout
is a smoke failure. `device_profile_mismatch` is acceptable route evidence when
the selected live device intentionally does not match the authored plan. P8AA
does not participate in normal runtime checks or static readiness-gate
execution, and it does not make default planner cutover, production route-level
probing parity, or Python planner deletion ready. The
`real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over` blockers remain blocked.
See `../../docs/rust-live-probe-evidence-and-cutover-gap.md` for the
consolidated P8X-P8AA live-probe evidence, default-route, production-route, and
readiness-gate gap summary. See
`../../docs/adr/0004-default-route-live-probe-cutover-design.md` for the
accepted future default-route live-probe cutover design, and
`../../docs/rust-default-route-probe-request-response.md` for the intended
future default-route probe request/response shape. See
`../../docs/rust-production-equivalent-live-probe-smoke.md` for the P8AE
evidence bar for a future production-equivalent live probe smoke.

P8S adds optional/manual smoke evidence for the direct shadow-binary fixture
harness:

```bash
python3 tools/smoke_rust_detected_facts_fixture.py \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow>
```

The smoke creates temporary matching, mismatching, and explicit-context override
fixture files, invokes the supplied `emuchef-plan-shadow` binary directly, and
emits deterministic JSON. It does not expose fixture mode through Python CLI
routes, invoke ADB, run executor/apply behavior, touch Tauri/protocol behavior,
or participate in normal runtime checks or the static readiness gate.

P8U adds optional/manual smoke evidence for the Python `rust-experimental`
fixture-forwarding route:

```bash
python3 tools/smoke_rust_experimental_detected_facts_fixture.py \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow>
```

The smoke creates temporary matching and mismatching fixture files, invokes
`python3 -m emuchef plan --planner-backend rust-experimental` with
`--rust-detected-facts-json <path>`, and emits deterministic JSON. Successful
route cases must emit concise Python-compatible summary stdout; raw Rust JSON
stdout is classified as `stdout_json` and fails the route smoke. The
mismatching output-file case checks only that the temporary output file exists
and contains `device_profile_mismatch`; it does not parse YAML or include temp
paths, output paths, file contents, or full process output in the report. P8U is
separate from the direct P8S Rust fixture smoke and does not expose fixture mode
through default Python planning or `rust-shadow`, invoke ADB, run
executor/apply, touch Tauri/protocol behavior, participate in normal runtime
checks, change readiness gate blockers, or make Rust the default planner.

P8V adds a crate-local, non-live ADB probe foundation in `src/device_probe.rs`.
It models the future `adb [-s SERIAL] shell getprop` command as argv only and
parses supplied bracketed `getprop` output into `DetectedDeviceFacts` fields for
manufacturer, brand, model, Android release, and SDK. Empty or whitespace-only
serial values are treated as absent, empty property values remain absent, and
device tags remain empty. This parser foundation does not execute ADB, start
subprocesses, read environment variables, access the filesystem, access the
network, change shadow command behavior, change Python CLI behavior, alter
executor/apply or Tauri/protocol behavior, wire smoke runners, run in the static
readiness gate, or make Rust planner probing authoritative.
P8W adds a crate-local live ADB probe adapter foundation on top of that parser.
`AdbDeviceProbe` executes the modeled argv through an injectable command runner,
and `ProcessCommandRunner` is the device-probe production runner that starts a
host process. It executes argv directly without a shell wrapper, handles empty
argv as a stable launch failure, and keeps stable probe errors free of raw
stderr, OS errors, paths, durations, process ids, serial values, and other
host-specific details. P8X wires this adapter only into the direct shadow binary.
It does not change Python CLI behavior, executor/apply, Tauri/protocol, smoke
runners, normal runtime checks, readiness-gate execution, or default planner
ownership.

P8A adds an explicit developer-only bridge through the Python CLI for an
already-built shadow binary:

```bash
emuchef plan \
  --planner-backend rust-shadow \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

The Python CLI route requires `--rust-planner-bin` and never invokes Cargo. It
forwards `--authored-root`, `--device-plan`, explicitly supplied device context
flags, and repeated raw `--bind` values to the supplied binary in original
order. Omitted `--rust-shadow-output` and explicit `--rust-shadow-output
passthrough` pass Rust stdout, stderr, and exit code through directly, so the
default shadow output remains Rust JSON/text passthrough.

P8E adds an explicit Python-compatible formatter mode for the same dev-only
bridge:

```bash
emuchef plan \
  --planner-backend rust-shadow \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --rust-shadow-output python-compatible \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

In `python-compatible` mode, the Python CLI parses usable Rust
`PlanningResult` JSON and formats it with the visible Python planning summary
labels by default. `--verbose` emits structured YAML to stdout, and `--output`
writes structured YAML to the requested path while stdout remains the concise
summary. The YAML is produced from the Rust JSON mapping through Python's
`dump_yaml(...)` helper; this mode does not rebuild full Python planner domain
objects. If Rust emits usable planning JSON with a non-zero exit code, the CLI
formats the result and preserves the Rust exit code. Empty, invalid, or
non-planning-result stdout is reported as a compatibility-mode error.

`--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag`
values are accepted by the explicit Rust routes. The Python CLI forwards only
values explicitly supplied on the command line; it does not resolve or forward
synthetic/profile-derived context. `--adb`, `--serial`, `--ops`, and `--debug`
remain rejected for Rust routes. `--output` and `--verbose` are accepted only for
explicit `--rust-shadow-output python-compatible` or for the always-compatible
`rust-experimental` route.

P8G adds an explicit non-default migration route that reuses the same supplied
shadow binary invocation while defaulting to Python-compatible formatting:

```bash
emuchef plan \
  --planner-backend rust-experimental \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

`rust-experimental` requires `--rust-planner-bin`, never invokes Cargo, forwards
the same authored-root/device-plan/explicit-context/repeated-bind arguments as
`rust-shadow`, and formats usable Rust `PlanningResult` JSON through the
Python-compatible summary or YAML path by default. It allows `--verbose` and
`--output` because those flags use the compatibility formatting path. It also
accepts `--rust-detected-facts-json <path>` as an explicit local-fixture
cutover rehearsal input and forwards it unchanged to `emuchef-plan-shadow` as
`--detected-facts-json <path>`. It accepts `--rust-probe-adb-getprop`,
`--rust-adb-path <path>`, and `--rust-serial <serial>` as an explicit live-probe
cutover rehearsal input and forwards those exact argparse strings to
`emuchef-plan-shadow` as `--probe-adb-getprop`, `--adb-path <path>`, and
`--serial <serial>`. The fixture and live-probe inputs are mutually exclusive
detected-facts sources, and Python does not invoke ADB or validate/normalize the
forwarded ADB path or serial.
`--rust-shadow-output` is only valid with
`--planner-backend rust-shadow`; Python and `rust-experimental` reject it before
ADB resolution or Python planner/session construction. `rust-experimental` is an
explicit non-default migration route. Its name and behavior may change before
Rust becomes the default planner backend. It is not the default planner, not a
stable final public contract, not Python planner deletion, and not an
executor/apply/Tauri/protocol behavior change.

P8B adds a developer-only matrix smoke for that explicit Python CLI route:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow>
```

The smoke requires an already-built shadow binary, creates only planner-visible
temporary placeholders for matrix bindings, invokes `python -m emuchef plan
--planner-backend rust-shadow` for each current matrix scenario by default, and
emits a deterministic JSON route-invocation report. It is not output parity
evidence; P7P remains the Python planner API versus Rust planner-output
comparison evidence.

P8F uses the same smoke runner for explicit Python-compatible output-mode
evidence:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --rust-shadow-output python-compatible
```

In `python-compatible` smoke mode, successful scenarios must exit `0` and emit
concise Python-compatible planning summary stdout. Raw Rust JSON stdout remains
classified as `stdout_json` and fails the compatibility-mode smoke for
successful scenarios. YAML-like output may classify as `python_yaml`, but it is
not the default P8F matrix success expectation unless a future scenario
explicitly defines that expectation. The smoke runner default remains
`passthrough`, and default generated commands omit `--rust-shadow-output
passthrough` to preserve P8B behavior.

P8H uses the same smoke runner for `--planner-backend rust-experimental` without
renaming the tool:

```bash
python3 tools/smoke_rust_shadow_cli_matrix.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored \
  --rust-planner-bin <path-to-emuchef-plan-shadow> \
  --planner-backend rust-experimental
```

In `rust-experimental` smoke mode, generated commands omit
`--rust-shadow-output`, the effective output mode is Python-compatible, and
successful scenarios must exit `0` and emit concise Python-compatible planning
summary stdout. Raw Rust JSON stdout remains classified as `stdout_json` and
fails the P8H smoke for successful scenarios.

P8I adds a static readiness gate for future default Rust planner proposals:

```bash
python3 tools/check_rust_planner_cutover_readiness.py \
  --authored-root authored \
  --scenario-matrix tools/plan_parity_scenarios.json
```

The gate is stdlib-only and emits deterministic JSON. It verifies static
prerequisites, derives checked-in device-plan coverage from
`authored/device_plans/*.yaml` and `authored/device_plans/*.yml` filenames, and
lists manual evidence commands that must be run before a future default-cutover
PR. It does not run the comparison harness, smoke runner, Cargo, npm, ADB,
executor/apply, Tauri/protocol, network, artifact materialization, or
fixture/golden regeneration checks. Its top-level status remains `blocked` even
when static checks pass because Python remains the default planner owner and
default-route, real-device probing, detected-device profile mismatch warning
parity, executor/apply, and Python planner deletion blockers remain unresolved.

P8K extends the dev-only matrix evidence with optional explicit
`device_context` data per scenario. The schema accepts non-empty `manufacturer`
and `model` strings, a non-negative integer `android_version`, and non-empty
ordered `device_tags`; detected-device, `adb`, `serial`, and probing fields are
rejected. Empty `device_tags` lists are rejected because the existing P8J flag
surface cannot distinguish an explicit empty tag override from omitted tag
flags. Omitted context keeps the synthetic/profile-derived planner context,
supplied scalar fields override it, and supplied tags replace profile tags in
order. This does not add ADB/device probing, detected-device facts,
executor/apply behavior, Tauri/protocol behavior, Cargo fallback behavior,
fixture/golden regeneration, or Python planner deletion readiness.

P8L refines the static readiness classification for that evidence. The readiness
gate keeps optional `device_context` schema validation separate from
explicit-context coverage: `device_context: {}` is schema-valid, but coverage
requires at least one valid scenario with a meaningful supplied context field.
The former broad real-device context blocker is narrowed into two unresolved
default-cutover blockers, `real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over`. The top-level report
status remains `blocked`, and the gate still does not run live comparison,
smoke, Cargo, npm, ADB, executor/apply, Tauri/protocol, network, artifact, or
fixture/golden work.

P8M records the accepted future ownership decision in
`../../docs/adr/0003-rust-real-device-context-ownership.md`: Rust should own
real-device probing and detected-device profile mismatch warning parity for
future default Rust planner cutover.
P8AC records the accepted future default-route live-probe cutover design in
`../../docs/adr/0004-default-route-live-probe-cutover-design.md`: default-route
live probing should be Rust-owned, and P8X-P8AA migration evidence does not
clear the default-route probing or mismatch-warning blockers. P8N adds the
crate-local foundation for the
first implementation step: `src/device_probe.rs` defines detected facts, stable
probe errors, a probe trait, a fake probe, and a helper that applies detected
facts over planner `DeviceContext`. P8O adds crate-private fake/test-backed
planner-input construction that applies detected facts over
synthetic/profile-derived context. P8P adds crate-private pure/test-backed
detected-device profile mismatch warning construction for supplied detected
facts and authored profile criteria. P8P evaluates the current Python warning
criteria for manufacturer, brand, model regex, and Android minimum version;
authored Android maximum values are parsed but not evaluated because the current
Python warning path does not evaluate them. P8Q composes those fake/test-backed
pieces into crate-private `PlanningResult` construction with detected context
and optional `device_profile_mismatch` warnings. P8R exposes that composition
path only through a local `emuchef-plan-shadow --detected-facts-json <path>`
fixture harness. The intended future precedence is synthetic/profile context ->
detected facts -> explicit CLI overrides. P8R applies explicit context overrides
to the emitted fixture-derived device context, while mismatch warnings remain
based on the detected fixture facts. P8R does not implement live ADB probing,
`rust-shadow` or `rust-experimental` route-level detection, executor/apply,
Tauri/protocol, Cargo fallback, fixture/golden, network, artifact,
runtime-check behavior, readiness gate reclassification, or Python planner
deletion readiness. P8T adds only explicit `rust-experimental` forwarding for a
local fixture path through the Python wrapper flag; it does not add live
probing, normal runtime checks, readiness gate reclassification, or default
backend behavior. P8V adds a pure `getprop` command model and supplied-output
parser in `src/device_probe.rs`; P8W adds the crate-local live adapter
foundation on top of that model and parser. P8X wires live probing only into the
direct dev-only Rust shadow binary, and P8Y adds optional/manual direct-shadow
smoke evidence for that path. The expected implementation path remains
incremental: explicit non-default production route support, readiness
reclassification, and a later default planner backend cutover.

P8C guards the explicit bridge's default CLI output compatibility contract. P7P
is planner DTO/result comparison evidence, P8B is Python CLI `rust-shadow` route
invocation evidence, P8C is the assertion that omitted
`--rust-shadow-output`/explicit `passthrough` remains dev-only Rust
stdout/stderr/exit-code passthrough, and P8E is the explicit formatter bridge for
usable Rust `PlanningResult` JSON. P8F is Python-compatible output-mode smoke
across the same explicit `rust-shadow` route and scenario matrix, P8G is the
explicit non-default `rust-experimental` migration route, and P8H is dev-only
matrix smoke evidence for that explicit route. P8I is static default-cutover
readiness reporting only. None of these makes Rust the default planner or makes
Python planner deletion ready.

P8D records the future default-route output compatibility decision in
`../../docs/adr/0002-rust-planner-cli-output-compatibility.md`. When Rust
eventually becomes the default planner backend for `emuchef plan`, default CLI
output and exit-code behavior must remain compatible with the current
Python-owned `emuchef plan` contract unless a separate accepted breaking-change
decision says otherwise. Python concise summaries, Python `--verbose`
structured YAML, and Python `--output` YAML file behavior remain the
compatibility targets. Rust-native JSON requires a future explicit
structured-output mode such as `--format json`; P8E's `python-compatible` mode
is a dev-only formatter bridge and does not make Rust default.

Bindings use the explicit form:

```bash
--bind app.retroarch.provision/retroarch_cfg=/tmp/retroarch.cfg
```

For this shadow slice, binding values are strings only. Repeated binds for the
same ref are grouped into a string array because the current Python CLI parser
groups repeated `--bind REF=VALUE` entries that way. This is dev-only shadow
behavior, not full future Rust planner CLI binding type parity.

The shadow command and Python bridge do not replace the default Python
`emuchef plan` CLI. They do not run executor/apply, inspect or probe devices,
invoke ADB, emit detected-device profile mismatch warnings, access the network,
download or materialize artifacts, regenerate checked-in goldens, expose Tauri
commands, expose sidecar protocol requests, invoke Cargo from the Python CLI
route, or alter the default
`emuchef-rust-backend` sidecar binary. Default Rust planner routing remains
blocked on broader output/behavior compatibility with the current Python CLI
contract or a separate accepted breaking-change decision, plus Rust-owned
real-device probing and detected-device profile mismatch warning parity as
accepted in ADR 0003. P8N's fake probe module and P8P's pure mismatch-warning
helper, plus P8Q's fake/test-backed result-composition helper, are not part of
Python CLI routes. P8R invokes that helper only from a local shadow-binary
fixture path and does not change production route behavior.
`default-run =
"emuchef-rust-backend"` is set so existing `cargo run --manifest-path
crates/emuchef-rust-backend/Cargo.toml -- --sidecar` and one-shot development
workflows remain unambiguous.

## Python Planner API Vs Rust Shadow Comparison

The repository includes a developer-only comparison/reporting harness:

```bash
.venv/bin/python tools/compare_rust_python_plan.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base
```

It also includes a dev-only current scenario matrix for checked-in device plans:

```bash
.venv/bin/python tools/compare_rust_python_plan.py \
  --scenario-matrix tools/plan_parity_scenarios.json \
  --authored-root authored
```

This compares Python planner API output with Rust `emuchef-plan-shadow` output.
It does not compare Python CLI behavior, because the user-facing
`emuchef plan` CLI probes device facts through ADB before planning. The Python
worker uses `load_authored_catalog(...)`, `Planner.start_session(...)`,
optional `session.bind_input(...)`, and `session.emit_execution_plan()` under a
shared synthetic/profile-derived planner context unless a matrix scenario
supplies explicit P8K `device_context` fields. For those scenarios, the harness
passes supplied values to both the hidden Python worker and the Rust shadow
command. Reports include stable context presence/key metadata only, not full
context values. This does not prove full Python CLI or real-device parity.

The report is deterministic JSON. It classifies differences as `match`,
`rust_missing`, `python_missing`, `value_mismatch`, `known_gap`,
`intentional_shape_difference`, or `unsupported`, and compares top-level status,
selected recipe refs, expanded recipe refs, execution step count, step ids and
order, step types, dependencies, normalized params, warning/error shape, and
serialized `permission_plan` presence.

For P7P, `match` means only that the dev-only comparison harness found no
unclassified differences for the compared fields under the supplied
planner-only bindings and shared planner context. It does not mean Python CLI
parity, real-device parity, executor/apply parity, artifact/network/
materialization parity, full schema parity, future scenario parity, or Rust
planner cutover readiness by itself.

Planner cutover readiness and Python planner deletion blockers are classified
in `../../docs/rust-planner-cutover-readiness.md`. The readiness document is
the source for future user-facing routing, matrix-gating, and Python deletion
criteria. The future default planner CLI output and exit-code compatibility
decision is classified in
`../../docs/adr/0002-rust-planner-cli-output-compatibility.md`. The future
default planner real-device context ownership decision is classified in
`../../docs/adr/0003-rust-real-device-context-ownership.md`.

By default the harness launches Rust with offline Cargo:

```bash
cargo run --offline --quiet --manifest-path crates/emuchef-rust-backend/Cargo.toml --bin emuchef-plan-shadow -- ...
```

Offline mode may fail on a fresh checkout without prefetched Cargo
dependencies. Pass `--cargo-online`, set
`EMUCHEF_PLAN_COMPARE_CARGO_OFFLINE=0`, or pass `--rust-bin <path>` to use a
prebuilt shadow binary.

Bindings that require existing local paths must use planner-only placeholders
that satisfy Python validation:

```bash
mkdir -p /tmp/emuchef-p7n-bios
: > /tmp/emuchef-p7n-xaniteog.apk
.venv/bin/python tools/compare_rust_python_plan.py \
  --authored-root authored \
  --device-plan ayaneo.pocket_s2.base \
  --bind feature.copy_bios/bios_source_dir=/tmp/emuchef-p7n-bios \
  --bind app.xaniteog.install/xaniteog_apk=/tmp/emuchef-p7n-xaniteog.apk
```

Matrix mode creates required planner-only temp directories and files from
`tools/plan_parity_scenarios.json` and includes binding refs/kinds plus stable
context presence/key metadata, not generated temp path values or full context
values, in the matrix report. It exits `0` only when every scenario's actual
classification matches its expected classification. This is an expectation check
for the current dev-only scenario matrix, not a claim of full planner
correctness. Future scenarios may intentionally expect `known_gap`; currently
all six checked-in scenarios are expected `match`:

| Scenario id | Device plan | Matrix bindings | Expected classification |
| --- | --- | --- | --- |
| `ayaneo_konkr_pocket_fit_base` | `ayaneo.konkr_pocket_fit.base` | None. | `match` |
| `ayaneo_pocket_s_mini_base` | `ayaneo.pocket_s_mini.base` | None. | `match` |
| `ayaneo_generic_base` | `ayaneo.generic.base` | `feature.copy_bios/bios_source_dir` directory. | `match` |
| `ayaneo_pocket_air_mini_base` | `ayaneo.pocket_air_mini.base` | `feature.copy_bios/bios_source_dir` directory. | `match` |
| `ayaneo_pocket_s2_base` | `ayaneo.pocket_s2.base` | `feature.copy_bios/bios_source_dir` directory; `app.xaniteog.install/xaniteog_apk` `.apk` file. | `match` |
| `ayaneo_pocket_s_mini_base_explicit_context` | `ayaneo.pocket_s_mini.base` | None; supplies explicit manufacturer, model, Android version, and ordered device tags. | `match` |

The Pocket S2 comparison currently reports `match` when required planner-only
BIOS and XaniteOG bindings are supplied. The comparison harness still has a
synthetic unit test for stale or future Rust outputs that return the old
`rust_optional_step_pruning_dependency_bug`, but that diagnostic is not a
current checked-in repo-plan comparison gap.

The comparison harness is not part of normal Rust/Tauri runtime checks. It does
not execute plans, probe devices, invoke ADB, access the network, download or
materialize artifacts, regenerate checked-in goldens, expose Tauri commands, or
alter user-facing CLI routing. Planner cutover remains a future explicit phase.
Single-scenario and matrix runs may require Python dependencies and Rust build
artifacts; they are developer tools, not user-facing planner paths.

The CLI-route smoke uses the same current scenario matrix but does not call this
comparison harness. It proves that explicit Python CLI Rust planner migration
routes can invoke the supplied Rust shadow planner across the current matrix.
The default smoke remains the `rust-shadow` raw passthrough route. P8F uses
`rust-shadow` with explicit `--rust-shadow-output python-compatible` and
requires concise Python-compatible summary stdout for successful scenarios. P8G
adds the explicit non-default `rust-experimental` route. P8H uses the same smoke
runner to exercise `--planner-backend rust-experimental`; generated commands omit
`--rust-shadow-output`, the effective output mode is Python-compatible, and
successful scenarios require exit `0` plus concise Python-compatible summary
stdout. P8K forwards optional scenario `device_context` values through these
smoke commands and records only stable context presence/key metadata. P8C
separately guards the `rust-shadow` default output contract as Rust passthrough.
None of these tools is wired into normal Rust/Tauri runtime checks, and none
changes default `emuchef plan` ownership. The P8I static readiness gate adds
deterministic reporting around these prerequisites and remaining blockers; it
lists manual evidence commands without executing them and remains `blocked`
until future phases intentionally clear default-cutover blockers.

## Phase 6O Executor Scope

Phase 6O added `src/executor.rs`, an internal Rust module used only by
crate-local tests at that phase. It is still not a public crate API, protocol
request, Tauri command, TypeScript API, backend selector, config option,
environment toggle, or public dry-run surface. Phase 6S later adds a minimal
crate-local `apply --dry-run` CLI parity path over selected fixtures only. The
parity target for Phase 6O itself is Python's internal
`ExecutorRunner.run(...)->ExecutionRunResult` value from `src/emuchef/executor`,
not CLI progress text, CLI summary text, `DryRunAdb.commands`, or sidecar
envelopes.

Python dry-run is dependency injection rather than a distinct result type:
`ExecutorRunner` receives `DryRunAdb` and, for CLI dry-run, a no-op sleep
function. Phase 6O mirrors that internal shape for selected fixtures. Step
statuses remain Python's `executed`, `skipped`, `blocked`, and `failed`; there is
no Rust-only dry-run status. Executor output parity is functional/semantic for
selected fixtures. Byte-for-byte JSON equality is useful in tests but is not a
general guarantee.

The Phase 6O runner preserves Python's supplied plan order and does not re-plan,
re-toposort, infer dependencies, mutate the input `ExecutionPlan`, or mutate
fixture documents. Failed and blocked dependency steps block dependents; skipped
steps do not. The only step handlers modeled are fixture-backed `wait` and
Python dry-run-shaped `grant_permissions`. The only condition types modeled are
the Python-backed executor conditions `package_installed`, `path_exists`, and
`file_exists`, evaluated against fake dry-run device state.

The executor adapters are test doubles. The fake device adapter may record
dry-run-shaped commands so tests can prove the adapter boundary was crossed, but
those records are not part of parity output because Python `ExecutionRunResult`
does not include `DryRunAdb.commands`. `run_plan_command` is a fake dry-run
adapter method only in Phase 6O. It never calls subprocesses, shell commands,
ADB, devices, permission APIs, or platform APIs.

Phase 6O intentionally does not implement real filesystem, network, archive,
copy, ADB, device, permission, launch, force-stop, subprocess, packaging, Python
bundling, or Tauri integration behavior. It also does not add executor request
routing, capability strings, a one-shot command, a JSONL command, protocol
negotiation, or unimplemented capability reporting. Later phases broaden the
same internal executor in fixture-backed slices. The final migration direction
remains a hard Rust cutover after feature parity; no backend selector or
long-term dual-backend toggle will be added.

## Phase 6P Executor Filesystem/Artifact Scope

Phase 6P keeps `src/executor.rs` internal to crate-local tests and adds selected
Python-compatible filesystem/artifact behavior behind explicit sandbox roots:

- `runtime_root` for `.emuchef_runtime` downloads and extraction output
- `cache_root` for `.emuchef_cache/artifacts`
- `fake_device_root` for `copy_files` fixture output paths such as `/sdcard/...`
- read-only fixture roots that may be used as source files

Every read or write must be classified under one of those roots. Writes are
accepted only under the runtime root, cache root, or fake device root. The fake
device root is a temp filesystem mapping for `copy_files` fixture parity only;
it is not an ADB adapter, install target, permission target, app lifecycle
surface, or a design for Phase 6Q device semantics.
Phase 6P sandbox operations reject symlink paths instead of trying to preserve
or follow them because no Python fixture currently defines symlink semantics.

The selected Phase 6P handlers are:

| Step type | Phase 6P status | Notes |
| --- | --- | --- |
| `resolve_artifacts` | Implemented for local/pre-cached no-network fixtures | Local `file://` URLs are copied only from allowed read-only fixture/temp roots into the runtime/cache root. Python's filename algorithm is mirrored: `cache: none` uses `sha256(artifact_id + url)-filename`; `cache: default` uses `sha256(url)-filename`. Existing default-cache files are treated as cache hits without network access. Missing remote URLs are not downloaded. |
| `extract_artifacts` | Implemented for host ZIP fixtures | Extracts resolved local artifact ZIPs under `runtime_root/extract/<step_id>/<artifact_basename>` and returns Python-shaped host `path_list` outputs. Device extraction remains out of scope. |
| `extract_archive` | Implemented for host ZIP fixtures | Extracts host-side ZIP files under `runtime_root/extract/<step_id>` and returns Python-shaped `extracted_path` values. Device extraction and cleanup of device temp files remain out of scope. |
| `copy_files` | Implemented for host-to-fake-device fixtures | Supports host `file_path`, `directory_path`, and `path_list` sources with fixture-covered `merge`, `sync`, and `replace` behavior. `sync` preserves stale destination files like Python `push_sync`; `replace` deletes only under the fake device root. Device sources and app-private/device semantics remain out of scope. |

Archive extraction intentionally hardens Python behavior. Python currently uses
`zipfile.ZipFile.extractall`, which does not enforce the Phase 6P sandbox
boundary by itself. Rust rejects absolute archive entries and `..` traversal
entries before writing anything. This is a safety hardening, not a public
behavioral divergence, because the Rust executor is still internal-only and all
Phase 6P filesystem behavior is test-contained.

Phase 6P did not add protocol/API/CLI executor routes, capability
strings, Tauri editor integration, backend selectors/toggles, protocol
negotiation, production packaging, Python bundling, hard cutover behavior, or
Python backend deletion. Real network downloads, HTTP clients, subprocesses,
ADB/device operations, permission grants, app launch/force-stop, and APK install
behavior remain unimplemented.

## Phase 6Q Executor Fake-Device/ADB Scope

Phase 6Q keeps the executor internal to `src/executor.rs` and adds selected
Python `DryRunAdb` parity for device/app/permission fixtures. It is still not
real-device executor parity. Real ADB/device/app/permission execution, manual
device testing, and integration parity remain deferred to Phase 6R.

The Python `DryRunAdb` state mirrored by the Rust fake adapter is intentionally
small: `installed_packages`, `remote_paths`, `remote_dirs`, and an internal
`commands` log. The mirrored dry-run methods are `install_apk`, `package_installed`,
`path_exists`, `path_is_dir`, `launch_app`, `force_stop_app`, and
`run_plan_command` for permission commands. `install_apk.replace_existing`
affects only the internal dry-run command record; Python `ExecutionRunResult`
still returns `{}` for install success, and Rust does the same.

The selected Phase 6Q executor behavior is:

| Step/area | Phase 6Q status | Notes |
| --- | --- | --- |
| `grant_permissions` | Expanded fixture parity | Runtime permissions, appops, rooted/API `when` filters, `not_applicable`, optional failures, fail policy, `require_all`, partial `permission_results`, and dependency blocking are covered. Permission intent remains step-local; top-level permissions are still invalid authored data. |
| `install_apk` | DryRunAdb parity only | Requires the same executor-layer host `file_path` runtime value, `.apk` suffix, and existing path checks as Python. Records fake install commands internally and does not mutate fake installed-package state. |
| `launch_app` | DryRunAdb parity only | Records fake launch commands internally and returns `{}` on success. It does not resolve activities, inspect installed packages, launch apps, or call real ADB. |
| `force_stop_app` | DryRunAdb parity only | Records fake force-stop commands internally, preserves Python's blank-package executor error, and returns `{}` on success. It does not stop real apps. |
| `skip_if` / `verify` | Python-backed condition parity | Supports `package_installed`, `path_exists`, and `file_exists` only, evaluated against fake dry-run state. `file_exists` means path exists and is not in `remote_dirs`; missing paths fail existence checks. |

Phase 6Q does not add real ADB traits, configs, environment variables, device
discovery, subprocess execution, network calls, manual harnesses, production
packaging, Python bundling, hard cutover behavior, Python backend deletion,
protocol/API executor routes, capability strings, public fake-device or
dry-run surfaces, Tauri editor integration, or backend selectors/toggles.
Python remains the legacy/reference implementation until parity or retirement is
confirmed.

## Phase 6R Real-ADB Adapter Foundations

Phase 6R keeps the executor internal and adds crate-private foundations for
real ADB execution. It does not add an executor protocol request, one-shot
request, JSONL request, Tauri integration, public dry-run surface, backend
selector, runtime toggle, production packaging, Python bundling, or hard cutover
from Python. Phase 6S later adds only a crate-local dry-run CLI parity path; it
does not expose real ADB execution. The default executor construction used by
normal tests continues to use the fake dry-run device adapter.

The Python command inventory mirrored by the Rust adapter is source-backed from
`src/emuchef/executor/adb.py` and related handlers:

| Area | Python-backed command shape |
| --- | --- |
| Host process execution | `subprocess.run(args, check=False, text=True, capture_output=True)` with an argv list, never a host shell. |
| Serial selection | Insert `-s <serial>` immediately after the ADB executable when a serial is configured. `run_plan_command` injects `-s <serial>` only when the command begins with `adb` and the tail does not already begin with `-s`. |
| Package check | `adb [-s serial] shell pm path <package>` with `check=False`; installed means exit code `0` and stdout containing `package:`. |
| Path check | `adb [-s serial] shell "<shlex-joined test -e command>"` with `check=False`; app-private paths wrap the inner command with Python-compatible `su -c` quoting. |
| File check | `path_exists(path) && !path_is_dir(path)`, matching Python condition behavior. |
| APK install | `adb [-s serial] install [-r] <apk-path>`. |
| Runtime permission | `adb [-s serial] shell pm grant <package> <permission>`. |
| Appop | `adb [-s serial] shell appops set <package> <op> <mode>`. |
| Launch explicit activity | `adb [-s serial] shell am start -n <package>/<activity>`. |
| Launch resolved activity | Try `shell cmd package resolve-activity --brief <package>`, then `shell pm resolve-activity --brief <package>`, then `shell am start -n <resolved-component>`. |
| Launch fallback | `adb [-s serial] shell monkey -p <package> -c android.intent.category.LAUNCHER 1`. |
| Force stop | `adb [-s serial] shell am force-stop <package>`. |

Rust keeps host-side ADB invocation as structured argv. For device-side `adb
shell` snippets, Rust constructs the same single shell payload string that
Python builds with `shlex.join`; it does not split or reinterpret that shell
snippet after construction. Unit tests cover spaces, quotes, backslashes,
`$`, glob characters, leading dashes, and nested `su -c` quoting for
app-private paths.

Phase 6R does not add a production timeout wrapper around ADB commands because
the Python reference path does not pass a timeout. Normal tests simulate
missing-binary and non-zero behavior through the fake ADB executor/result layer;
they do not attempt to launch a missing executable with `std::process::Command`.
The real process runner is ADB-specific and is the only code path that uses
`std::process::Command`.

Real-device tests are ignored/manual only. They are compiled with crate tests
but are marked `#[ignore]`, require `EMUCHEF_RUN_REAL_ADB_TESTS=1` before
constructing a real ADB device, and never run as part of normal `cargo test`.
Environment variables listed below are test-only controls and are not runtime
backend selectors:

| Variable | Purpose |
| --- | --- |
| `EMUCHEF_RUN_REAL_ADB_TESTS=1` | Global manual-test opt-in required by every real-device test. |
| `EMUCHEF_TEST_DEVICE_SERIAL=<serial>` | Optional serial passed explicitly to the manual-test `RealAdbDevice`. |
| `EMUCHEF_TEST_PACKAGE=<package>` | Test-owned package used by package, launch, force-stop, permission, or appops tests. |
| `EMUCHEF_TEST_DEVICE_PATH=<path>` | Optional benign path for the manual path-exists check; defaults to `/sdcard`. |
| `EMUCHEF_TEST_APK=<path>` | Explicit test-owned APK path for the install test. |
| `EMUCHEF_TEST_ACTIVITY=<activity>` | Optional explicit activity for the launch test. |
| `EMUCHEF_TEST_PACKAGE_ALLOWLIST=<package>` | Required exact allowlist for force-stop, permission, and appops tests. Comma-separated values are accepted. |
| `EMUCHEF_TEST_RUNTIME_PERMISSION=<permission>` | Runtime permission used only by the permission manual test. |
| `EMUCHEF_TEST_APPOP=<op>` and `EMUCHEF_TEST_APPOP_MODE=<mode>` | Appop and mode used only by the appops manual test. |
| `EMUCHEF_RUN_REAL_ADB_INSTALL_TEST=1` | Per-test opt-in for installing the explicit test APK. |
| `EMUCHEF_RUN_REAL_ADB_LAUNCH_TEST=1` | Per-test opt-in for launching the explicit test package. |
| `EMUCHEF_RUN_REAL_ADB_FORCE_STOP_TEST=1` | Per-test opt-in for force-stopping the explicit allowlisted test package. |
| `EMUCHEF_RUN_REAL_ADB_PERMISSION_TEST=1` | Per-test opt-in for granting the explicit permission to the allowlisted test package. |
| `EMUCHEF_RUN_REAL_ADB_APPOPS_TEST=1` | Per-test opt-in for applying the explicit appop to the allowlisted test package. |

Permission and appops manual tests refuse system-like packages such as
`android`, `android.*`, `com.android.*`, and `com.google.android.*`. They are
intended only for operator-provided test packages. Manual tests do not wipe
device data, reboot devices, uninstall packages, modify global settings, perform
network downloads, or clean up arbitrary packages.

## Document Session Scope

Phase 6J.2 document sessions are process-local JSONL sidecar state. Opening a
recipe loads the Phase 6E authored recipe model, emits canonical YAML, records
that YAML as the saved baseline, and returns a Python-shaped `RecipeDocumentDto`.
`applyRecipeCommand` supports the overview commands added in Phase 6G:

```json
{"type":"SetOverviewField","field":"name","value":"New Name"}
{"type":"SetOverviewField","field":"description","value":"New description"}
{"type":"SetOverviewField","field":"description","value":null}
```

Recipe `id`, `kind`, `schemaVersion`, and `schema_version` are read-only in this
Rust backend slice and are rejected. `description: null` matches Python
behavior: it clears the description, projects the DTO description as an empty
string, and omits the top-level `description:` key from canonical YAML. Empty or
whitespace-only `description` values also clear the field. Empty or
whitespace-only `name` values fail command execution.

Phase 6I expands `applyRecipeCommand` to these non-step command families:

```text
AddInput
RenameInput
UpdateInputField
DeleteInput
DuplicateInput
AddArtifact
UpdateArtifactField
RenameArtifact
DeleteArtifact
DuplicateArtifact
AddArtifactGroup
RenameArtifactGroup
DeleteArtifactGroup
DuplicateArtifactGroup
ReorderArtifactGroup
AddArtifactGroupMember
RemoveArtifactGroupMember
ReorderArtifactGroupMember
```

Input mutations cover the fields currently decoded by Python:

```text
type
role
label
description
required
multiple
validation.must_exist
validation.allowed_extensions
validation.path_kind
```

Artifact mutations cover Python's current authored `remote_file` fixture surface:
`url` and `cache`. Artifact `type` remains authored as `remote_file` in the Rust
model and is not editable through a Phase 6I command. This is fixture-scoped
artifact parity, not a claim of broader artifact kind support.

Phase 6J.1 expands `applyRecipeCommand` to these step lifecycle and dependency
command families:

```text
AddStep
DeleteStep
DuplicateStep
ReorderStep
UpdateStepBasics
SetStepUserToggleable
UpdateStepDependencies
```

`AddStep` creates the same verified authored step shape as Python for this
migration slice: normalized id/name, supported `stepType`, `user_toggleable:
false`, empty `dependencies`, empty `constraints`, empty `skip_if`, empty
`params`, empty `verify`, and no description. It does not materialize StepSpec
default params. `UpdateStepBasics` edits only `name` and `description`; `null`,
empty, and whitespace-only descriptions clear the description, project as an
empty DTO string, and omit `description:` from canonical YAML. Step `id` and
`type` remain read-only.

`UpdateStepDependencies` is a full replacement command. It preserves authored
order, normalizes dependency ids, rejects duplicate dependency ids, and
intentionally allows missing dependency targets because Python command
application allows them. Phase 6K validation reports local missing-target and
cycle diagnostics after the command applies; command application still does not
construct a planner graph or infer execution order.

Phase 6J.2 expands `applyRecipeCommand` to the remaining current external
Python step params and advanced internals command families:

```text
UpdateStepParams
UpdateStepConstraints
UpdateStepSkipIf
UpdateStepVerify
```

`UpdateStepParams` is a full replacement command. It preserves submitted param
object order in the in-memory model, converts only an immediate param value with
the exact JSON shape `{"ref":"..."}` and a string ref into an authored ref, and
keeps nested or list-contained ref-shaped objects literal. It removes builtin
StepSpec default literals only when the authored value compares equal under the
Python command semantics used by the editor. It does not infer missing params,
materialize defaults, validate ref existence, or add dependencies from refs.

`UpdateStepConstraints` is a full replacement command. The JSON command shape
uses `conflictsWith`; emitted authored YAML uses `conflicts_with`; DTOs continue
to use `conflictsWith`. Constraint object keys other than `capabilities` and
`conflictsWith` are rejected as `invalid_command`. Duplicate or blank-after-trim
identifiers decode successfully when they are non-empty JSON strings, then fail
application as `command_failed`, matching the Python command path.

`UpdateStepSkipIf` and `UpdateStepVerify` are full replacement commands for
condition lists. Each condition accepts only `type` and optional `params`;
unknown condition types pass through; `params` defaults to an empty object and
must be an object when present. Ref-shaped values inside condition params remain
literal JSON/YAML values and do not affect dependencies or RefIndex.

The current external Python `command_codec.py` `_DECODERS` inventory contains 30
command types. Phase 6J.2 supports all 30 command type strings through Rust
`applyRecipeCommand`. Internal/core-only Python command dataclasses that are not
present in `_DECODERS` are not external sidecar commands and remain outside this
Rust backend slice: recipe dependency list commands, provided feature list
commands, `RenameRecipeIdCommand`, and `RenameStepCommand`.

Input, artifact, and artifact group rename/delete commands perform the same
tested safe cleanup as Python for supported immediate step params:

- input refs such as `inputs.source_dir`
- artifact field refs such as `artifacts.target_zip.local_path`
- supported `artifacts` string-list params
- supported `artifact_groups` string-list params
- artifact group membership when artifacts are renamed or deleted

`DeleteStep` performs only the Python-verified cleanup targets that the Rust
model already represents faithfully for Phase 6J.1 fixtures: other steps'
`dependencies`, `constraints.conflicts_with`, and supported top-level step param
refs such as `steps.prepare` and `steps.prepare.outputs.extracted_path`.

Nested literal ref-shaped data, `skip_if`, `verify`, unsupported step types, and
custom params remain preserved instead of recursively rewritten. Step delete
does not expand skip/verify or advanced internals cleanup in Phase 6J.2.
Artifact groups are not RefIndex sources; group mutations do not create
`artifact_groups.*` refs.

Changing commands regenerate canonical YAML first, then rerun the Phase 6K
editor-local validation checks for the current in-memory recipe. No-op commands return
`changed: false` and do not push undo history. Invalid commands leave the
stored document unchanged. Successful changing commands also return a refreshed
document DTO, including current dirty/canUndo/canRedo state and a RefIndex
derived from the mutated recipe.

`saveRecipe` writes the current canonical YAML back to the document's current
path, updates the saved baseline after a successful write, reruns Phase 6K
editor-local validation, preserves undo/redo stacks, and returns the current
document DTO. Save tests must use temporary copies of fixtures; do not point save
tests at checked-in fixture files.

Undo/redo use content snapshots of the recipe model and current canonical YAML,
then rerun validation after restoring changed content. The saved baseline is not
part of content snapshots; dirty state is always recalculated from the current
canonical YAML versus the most recent opened/saved baseline. Undo and redo on
empty stacks match Python behavior by returning `changed: false` success
responses with the current document.

`emitYaml` returns the current in-memory canonical YAML for an open document.
`validate` reruns Phase 6K editor-local validation for the current in-memory
recipe and returns diagnostics only.

`getRefIndex` returns generated RefIndex data for the current in-memory document
state. `RecipeDocumentDto.refIndex` is no longer the Phase 6F empty placeholder
for modeled Phase 6H recipe features. The generated index is fixture-scoped and
limited to the currently modeled Rust authored recipe data: inputs, runtime
artifact fields, authored step ids, and declared StepSpec outputs from the
Rust-owned StepSpec metadata. It does not derive planner, catalog-context,
executor/device, artifact group, recipe `provides`, or missing-param refs.

```json
{
  "inputRefs": ["inputs.bios_source_dir"],
  "artifactRefs": [],
  "stepRefs": ["steps.copy_bios_dir"],
  "stepOutputRefs": ["steps.copy_bios_dir.outputs.copied_paths"],
  "allRefs": [
    "inputs.bios_source_dir",
    "steps.copy_bios_dir",
    "steps.copy_bios_dir.outputs.copied_paths"
  ],
  "candidates": [
    {
      "ref": "inputs.bios_source_dir",
      "label": "Input · bios_source_dir",
      "valueType": "directory_path",
      "sourceKind": "input",
      "sourceId": "bios_source_dir"
    }
  ]
}
```

## StepSpec Source

`listStepSpecs` returns Rust-owned static StepSpec DTO metadata from
`src/step_specs.rs`. The data includes editor-safe labels, supported status,
primary outputs, param ordering, defaults, ref filters, enum values, and known
param shape metadata for the built-in step types.

The Rust backend does not embed a Python-generated StepSpec fixture and normal
Rust/Tauri runtime tests do not invoke Python for StepSpec metadata. Python
still retains broader CLI, planner, executor, editor API/reference, and other
golden-generation responsibilities until those surfaces are replaced or retired
explicitly.

The fixture stores only the Python response `result` object, not the outer
`{"ok": true, "result": ...}` envelope.

## Authored YAML Scope

`emitRecipeYamlFromPath` loads an authored recipe YAML file, builds the narrow
Phase 6E Rust model, and emits canonical authored YAML. Semantic YAML parity with
Python is the priority. Byte-for-byte parity is best effort and is only asserted
where stable; broader fixture tests compare parsed YAML semantics so harmless
emitter formatting does not block the phase.

The Rust loader supports these authored recipe sections:

- `schema_version`
- `kind`
- `id`
- `name`
- `description`
- `recipe_dependencies`
- `provides`
- `inputs`
- `artifacts`
- `artifact_groups`
- `steps`

Top-level `permissions` is rejected. Permission intent remains step-local through
`grant_permissions.params`.

Step params treat a value as a ref only when the immediate param value is a map
with exactly one key, `ref`, and that value is a string. Nested ref-shaped maps in
lists, objects, `skip_if`, or `verify` remain literal authored data.

`validateRecipePath` returns the Python-compatible result shape:

```json
{"diagnostics":[]}
```

For Phase 6K and Phase 6L fixtures it targets functional/semantic parity with Python
editor-facing diagnostics. Structured diagnostic fields are the primary parity
contract: `severity`, `code`, `objectKind`, `objectId`, `field`, and diagnostic
presence. Byte-for-byte `message` equality is best effort and not required when
the message has the same meaning.

Phase 6K editor-local validation covers only data available from the loaded
recipe, embedded StepSpecs, and local authored refs:

- load/schema shape diagnostics for the current Rust YAML model
- removed top-level `permissions`
- local authored step dependency missing-target and cycle diagnostics
- StepSpec missing required params and literal/ref mode checks
- exact top-level `ParamValue::Ref` validation for inputs, artifacts, artifact
  runtime fields, steps, and primary step outputs
- nested ref-shaped literals in params, `skip_if`, and `verify` remaining
  literal data

Known Phase 6K limits:

- Malformed YAML parser messages come from `serde_yaml`, so wording and source
  spans can differ from PyYAML. The diagnostic shape, severity, code, file, and
  object fields are still matched.
- Broad planner and executor validation remain incomplete. Phase 6N planner
  behavior is limited to Python-golden-backed internal fixtures and does not
  replace executor validation.
- Broad built-in plugin hook validation is not complete. Phase 6K implements only
  fixture-required local checks; omitted plugin-hook diagnostics remain future
  work.
- Python remains the legacy/reference implementation until parity or retirement
  is confirmed. The project direction is a hard Rust cutover after parity, not a
  user-facing backend selector or long-term dual-backend toggle.

Phase 6L authoredRoot/catalog-context validation covers the Python-verified
recipe diagnostics needed by the focused Rust fixtures:

- `validateRecipePath` uses an explicit non-null `authoredRoot` as catalog
  context and does not infer it from the recipe path. Omitted `authoredRoot` and
  `authoredRoot: null` keep the `validation_context_limited` warning.
- `openRecipe` normalizes an explicit repo root containing `authored/recipes` to
  that `authored` directory, accepts an explicit `authored` directory, and infers
  `authoredRoot` from recipe files under an `authored/recipes` tree.
- sidecar `validate`, successful changing commands, undo, redo, and save reuse
  the document's stored authoredRoot. They do not re-infer a root after open.
- recipe dependency missing-target diagnostics use `recipe_not_found`.
- recipe dependency cycles use a small validation-local graph walk and report
  `dependency_cycle`; Rust does not introduce planner module naming, planner
  data structures, execution plans, step expansion, or apply-device behavior.
- duplicate recipe ids between the open document and another catalog recipe use
  `recipe_id_conflict`.

Phase 6L intentionally reads only this Python-verified top-level catalog
inventory:

```text
apps/*.y*ml
recipes/*.y*ml
device_profiles/*.y*ml
device_plans/*.y*ml
```

The Rust Phase 6L fixture implementation models only `recipes/*.yml` and
`recipes/*.yaml` data because no focused fixture needs app, device profile, or
device plan diagnostics. It still confines catalog discovery to the verified
top-level directory inventory and does not scan nested directories, templates,
alternate file extensions, runtime paths, device paths, network URLs, or Tauri
workspace metadata. Cross-file `provides.features` availability is not
implemented because Python validation does not emit provided-feature diagnostics
in this path. Duplicate recipe-id diagnostics are fixture-scoped to opened
recipes under the authored catalog root; temp copies opened outside that root do
not report catalog duplicate-id conflicts so earlier document-session fixture
flows remain stable.

## Python Goldens

Recipe fixtures live under:

```text
crates/emuchef-rust-backend/tests/fixtures/recipes/
```

Focused Phase 6L authoredRoot fixture trees live under:

```text
crates/emuchef-rust-backend/tests/fixtures/authored_root/
```

Python-generated parity fixtures live under:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/
```

Fixture/golden ownership, consumers, classifications, and retirement criteria
are tracked in `docs/python-fixture-golden-ownership.md`. Any Python
regeneration command in this README is dev-only/reference-only and is not part
of normal setup, runtime, packaging, or Rust/Tauri verification.

The former Phase 6E regeneration command used the removed Python editor API.
These checked-in goldens remain parity evidence. Refresh them only after adding
Rust-native fixture tooling or an explicitly retained non-editor Python owner.

The emitted YAML goldens store only the Python result string. The validation
goldens store only the Python result object, not the outer API envelope.
Phase 6L adds Python-generated diagnostic goldens named
`phase6l_*.diagnostics.json`. They store the semantic diagnostic fields the Rust
tests compare: severity, code, objectKind, objectId, and field.
Phase 6M adds `phase6m_planner_*.json` planning-result fixtures in this
directory. Phase 6N adds `phase6n_planner_*.json` planning-result fixtures for
the expanded private planner coverage. They are generated from Python
`Planner(...).start_session(...).emit_execution_plan()` using focused
authoredRoot recipe fixtures. New Phase 6N goldens normalize absolute repo-root
paths to `$REPO_ROOT/...` so relative input defaults and bindings remain stable.
Phase 6O adds `phase6o_executor_*.json` execution-result fixtures generated from
Python `ExecutorRunner(...).run(...)` with `DryRunAdb` and a no-op sleep
function. These goldens store only the internal Python `ExecutionRunResult`
object. They do not store CLI progress text, CLI summary text, sidecar
envelopes, or `DryRunAdb.commands`.

Dev-only/reference-only: regenerate the Phase 6O executor goldens from the repo
root with:

```bash
PYTHONPATH=src python3 - <<'PY'
from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from dataclasses import fields, is_dataclass
from enum import Enum
from pathlib import Path

from emuchef.domain import (
    DeviceContext,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    LiteralParamValue,
    RuntimeCapabilities,
    StepCondition,
)
from emuchef.executor import DryRunAdb, ExecutorRunner

GOLDENS = Path("crates/emuchef-rust-backend/tests/fixtures/python_goldens")
GOLDENS.mkdir(parents=True, exist_ok=True)

def to_primitive(value):
    if is_dataclass(value):
        return {field.name: to_primitive(getattr(value, field.name)) for field in fields(value)}
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Mapping):
        return {str(key): to_primitive(item) for key, item in value.items()}
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return [to_primitive(item) for item in value]
    return value

def caps() -> RuntimeCapabilities:
    return RuntimeCapabilities(
        adb_available=True,
        apk_install=True,
        shared_storage_write=True,
        app_launch=True,
        shell_command=True,
        package_remove_for_user=False,
        root_shell=True,
        app_data_write=True,
    )

def context() -> DeviceContext:
    return DeviceContext(
        manufacturer="Example",
        model="Example",
        android_version=13,
        android_api_level=33,
        device_tags=(),
    )

def plan(*steps: ExecutionStep) -> ExecutionPlan:
    return ExecutionPlan(
        id="plan.test",
        source=ExecutionPlanSource(
            device_profile_ref="example.device_profile",
            device_plan_ref="example.device_plan",
            selected_recipe_refs=("example.recipe",),
            expanded_recipe_refs=("example.recipe",),
        ),
        device_context=context(),
        runtime_capabilities=caps(),
        inputs=(),
        artifacts=(),
        steps=steps,
    )

def run(plan_value: ExecutionPlan, adb: DryRunAdb | None = None) -> dict:
    result = ExecutorRunner(adb=adb or DryRunAdb(), sleep_fn=lambda _: None).run(plan_value)
    return to_primitive(result)

fixtures = {
    "phase6o_executor_wait_success.json": run(plan(ExecutionStep(
        id="example.recipe/wait",
        recipe_ref="example.recipe",
        type="wait",
        name="Wait",
        params={"duration_ms": LiteralParamValue(value=10)},
    ))),
    "phase6o_executor_failure_blocking.json": run(plan(
        ExecutionStep(
            id="example.recipe/fail",
            recipe_ref="example.recipe",
            type="wait",
            name="Fail",
            params={"duration_ms": LiteralParamValue(value=0)},
        ),
        ExecutionStep(
            id="example.recipe/downstream",
            recipe_ref="example.recipe",
            type="wait",
            name="Downstream",
            dependencies=("example.recipe/fail",),
            params={"duration_ms": LiteralParamValue(value=1)},
        ),
        ExecutionStep(
            id="example.recipe/unrelated",
            recipe_ref="example.recipe",
            type="wait",
            name="Unrelated",
            params={"duration_ms": LiteralParamValue(value=1)},
        ),
    )),
    "phase6o_executor_grant_permissions.json": run(plan(ExecutionStep(
        id="example.recipe/grant",
        recipe_ref="example.recipe",
        type="grant_permissions",
        name="Grant",
        params={
            "runtime": LiteralParamValue(value=[{
                "package_name": "com.example.app",
                "name": "android.permission.POST_NOTIFICATIONS",
                "required": False,
            }]),
            "appops": LiteralParamValue(value=[{
                "package_name": "com.example.app",
                "op": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow",
                "required": False,
                "when": {"rooted": False},
            }]),
        },
    ))),
}

adb = DryRunAdb()
adb.installed_packages.add("com.example.skip")
fixtures["phase6o_executor_skip_if.json"] = run(plan(
    ExecutionStep(
        id="example.recipe/skipped",
        recipe_ref="example.recipe",
        type="wait",
        name="Skipped",
        skip_if=(StepCondition(type="package_installed", params={"package_name": "com.example.skip"}),),
        params={"duration_ms": LiteralParamValue(value=1)},
    ),
    ExecutionStep(
        id="example.recipe/downstream",
        recipe_ref="example.recipe",
        type="wait",
        name="Downstream",
        dependencies=("example.recipe/skipped",),
        params={"duration_ms": LiteralParamValue(value=1)},
    ),
), adb=adb)

class FailingPermissionAdb(DryRunAdb):
    def run_plan_command(self, command):
        super().run_plan_command(command)
        if tuple(command) == (
            "adb",
            "shell",
            "pm",
            "grant",
            "com.example.fail",
            "android.permission.CAMERA",
        ):
            raise RuntimeError("permission denied")

fixtures["phase6o_executor_grant_permissions_failure.json"] = run(plan(
    ExecutionStep(
        id="example.recipe/grant_fail",
        recipe_ref="example.recipe",
        type="grant_permissions",
        name="Grant Fail",
        params={
            "runtime": LiteralParamValue(value=[{
                "package_name": "com.example.fail",
                "name": "android.permission.CAMERA",
                "required": True,
            }]),
            "policy": LiteralParamValue(value={"on_failure": "warn", "require_all": False}),
        },
    ),
    ExecutionStep(
        id="example.recipe/dependent",
        recipe_ref="example.recipe",
        type="wait",
        name="Dependent",
        dependencies=("example.recipe/grant_fail",),
        params={"duration_ms": LiteralParamValue(value=1)},
    ),
    ExecutionStep(
        id="example.recipe/unrelated",
        recipe_ref="example.recipe",
        type="wait",
        name="Unrelated",
        params={"duration_ms": LiteralParamValue(value=1)},
    ),
), adb=FailingPermissionAdb())

for name, payload in fixtures.items():
    (GOLDENS / name).write_text(
        json.dumps(payload, indent=2, sort_keys=False) + "\n",
        encoding="utf-8",
    )
PY
```

Phase 6P adds these normalized executor goldens:

```text
phase6p_executor_resolve_extract_copy_flow.json
phase6p_executor_extract_archive_success.json
phase6p_executor_extract_archive_invalid_failure.json
```

They were generated with the same dev-only/reference-only
`PYTHONPATH=src python3 - <<'PY'` pattern, using Python
`ExecutorRunner(adb=DryRunAdb(), workdir=TemporaryDirectory())`.
The helper creates only disposable temp files, local `file://` ZIP fixtures, and
normalizes the temp root to `$TMP` before writing the JSON fixture. No Python
golden was generated for missing host-copy sources because Python `DryRunAdb`
records the push without checking host source existence; Rust's temp-root-backed
filesystem adapter validates real fixture sources and covers that safety behavior
with Rust-side tests instead.

Phase 6Q adds these normalized executor goldens:

```text
phase6q_executor_install_apk_replace_existing.json
phase6q_executor_launch_force_stop.json
phase6q_executor_device_app_failure_blocking.json
phase6q_executor_permission_partial_failure.json
phase6q_executor_file_dir_conditions.json
```

They were generated from the repo root with the same dev-only/reference-only
safe pattern:

```bash
PYTHONPATH=src python3 - <<'PY'
# Build ExecutionPlan values in memory, run ExecutorRunner(adb=DryRunAdb(),
# sleep_fn=lambda _: None), normalize temp paths to "$TMP", and write
# crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6q_executor_*.json.
PY
```

The Phase 6Q helper uses only temp-owned APK/text fixtures and Python
`DryRunAdb`. It does not call real ADB, probe devices, run app lifecycle
operations, grant permissions, perform network access, or mutate authored
fixtures.

The former Phase 6L diagnostic regeneration command used the removed Python
editor API. These checked-in diagnostic goldens remain parity evidence. Refresh
them only after adding Rust-native fixture tooling or an explicitly retained
non-editor Python owner.

Dev-only/reference-only: regenerate the Phase 6N planner goldens from the repo
root with:

```bash
PYTHONPATH=src:tests .venv/bin/python - <<'PY'
from __future__ import annotations

import json
import tempfile
from pathlib import Path

import yaml

from emuchef.domain import DeviceContext
from emuchef.io import load_authored_catalog
from emuchef.io.serde import to_primitive
from emuchef.planner import Planner
from support import build_authored_tree

REPO_ROOT = str(Path.cwd())
ROOT = Path("crates/emuchef-rust-backend/tests/fixtures/authored_root")
GOLDENS = Path("crates/emuchef-rust-backend/tests/fixtures/python_goldens")
GOLDENS.mkdir(parents=True, exist_ok=True)

CASES = [
    ("planner_phase6n_builtins_all", ["main.yaml"], ["planner.phase6n.builtins_all"], "phase6n_planner_builtins_all.json", None),
    ("planner_phase6n_optional_inputs", ["main.yaml"], ["planner.phase6n.optional_inputs"], "phase6n_planner_optional_inputs_omitted.json", None),
    ("planner_phase6n_optional_inputs", ["main.yaml"], ["planner.phase6n.optional_inputs"], "phase6n_planner_optional_inputs_bound.json", {"planner.phase6n.optional_inputs/optional_cfg": "relative/optional.cfg"}),
    ("planner_phase6n_input_defaults_multiple", ["main.yaml"], ["planner.phase6n.input_defaults_multiple"], "phase6n_planner_input_defaults_multiple.json", None),
    ("planner_phase6n_dependency_graph", ["dependency_a.yaml", "dependency_b.yaml", "main.yaml"], ["planner.phase6n.dependency_graph"], "phase6n_planner_dependency_graph.json", None),
    ("planner_phase6n_step_data", ["main.yaml"], ["planner.phase6n.step_data"], "phase6n_planner_step_data.json", None),
]

def normalize(value):
    if isinstance(value, dict):
        return {key: normalize(item) for key, item in value.items()}
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if isinstance(value, str) and value.startswith(REPO_ROOT + "/"):
        return "$REPO_ROOT/" + value[len(REPO_ROOT) + 1:]
    return value

for fixture, recipe_files, selected_recipe_refs, golden_name, bindings in CASES:
    recipes = []
    for recipe_file in recipe_files:
        with (ROOT / fixture / "authored" / "recipes" / recipe_file).open(encoding="utf-8") as handle:
            recipes.append(yaml.safe_load(handle))

    with tempfile.TemporaryDirectory() as tmp:
        authored_root = build_authored_tree(
            Path(tmp),
            recipes=recipes,
            selected_recipe_refs=selected_recipe_refs,
        )
        session = Planner(load_authored_catalog(authored_root)).start_session(
            "example.device_plan",
            DeviceContext(
                manufacturer="Example",
                model="Example",
                android_version=13,
                android_api_level=33,
                device_tags=(),
            ),
        )
        if bindings:
            for input_id, value in bindings.items():
                update = session.bind_input(input_id, value)
                if update.errors:
                    raise RuntimeError(update.errors)

        result = normalize(to_primitive(session.emit_execution_plan()))
        (GOLDENS / golden_name).write_text(
            json.dumps(result, indent=2, sort_keys=False) + "\n",
            encoding="utf-8",
        )
PY
```

Dev-only/reference-only: regenerate the Phase 6M planner goldens from the repo
root with:

```bash
PYTHONPATH=tests .venv/bin/python - <<'PY'
from __future__ import annotations

import json
import tempfile
from pathlib import Path

import yaml

from emuchef.domain import DeviceContext
from emuchef.io import load_authored_catalog
from emuchef.io.serde import to_primitive
from emuchef.planner import Planner
from support import build_authored_tree

ROOT = Path("crates/emuchef-rust-backend/tests/fixtures/authored_root")
GOLDENS = Path("crates/emuchef-rust-backend/tests/fixtures/python_goldens")
GOLDENS.mkdir(parents=True, exist_ok=True)

CASES = [
    ("planner_minimal", ["main.yaml"], ["planner.minimal"], "phase6m_planner_minimal.json", None),
    ("planner_dependencies", ["dependency.yaml", "main.yaml"], ["planner.dependencies"], "phase6m_planner_dependencies.json", None),
    ("planner_refs_artifacts", ["main.yaml"], ["planner.refs_artifacts"], "phase6m_planner_refs_artifacts.json", None),
    ("planner_inputs", ["main.yaml"], ["planner.inputs"], "phase6m_planner_inputs_bound.json", {"planner.inputs/required_cfg": "/tmp/example.cfg"}),
    ("planner_inputs", ["main.yaml"], ["planner.inputs"], "phase6m_planner_inputs_missing.json", None),
    ("planner_grant_permissions", ["main.yaml"], ["planner.grant_permissions"], "phase6m_planner_grant_permissions.json", None),
]

for fixture, recipe_files, selected_recipe_refs, golden_name, bindings in CASES:
    recipes = []
    for recipe_file in recipe_files:
        with (ROOT / fixture / "authored" / "recipes" / recipe_file).open(encoding="utf-8") as handle:
            recipes.append(yaml.safe_load(handle))

    with tempfile.TemporaryDirectory() as tmp:
        authored_root = build_authored_tree(
            Path(tmp),
            recipes=recipes,
            selected_recipe_refs=selected_recipe_refs,
        )
        session = Planner(load_authored_catalog(authored_root)).start_session(
            "example.device_plan",
            DeviceContext(
                manufacturer="Example",
                model="Example",
                android_version=13,
                android_api_level=33,
                device_tags=(),
            ),
        )
        if bindings:
            for input_id, value in bindings.items():
                update = session.bind_input(input_id, value)
                if update.errors:
                    raise RuntimeError(update.errors)

        result = to_primitive(session.emit_execution_plan())
        (GOLDENS / golden_name).write_text(
            json.dumps(result, indent=2, sort_keys=False) + "\n",
            encoding="utf-8",
        )
PY
```

Phase 6G document sessions are covered by Rust integration tests and focused
Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6g_*.result.json
```

Those goldens cover overview name changes, description changes,
`description:null`, no-op commands, empty undo/redo, undo/redo after mutation,
open-document `emitYaml`, and open-document `validate`. They normalize
`documentId`, paths, authored roots, and diagnostic files. The Phase 6G golden
recipe has no inputs, artifacts, or steps, so its generated RefIndex is empty
even after Phase 6H replaces the former placeholder behavior for richer
modeled recipes.

The former Phase 6G regeneration command used the removed Python editor API.
These checked-in result goldens remain parity evidence. Refresh them only after
adding Rust-native fixture tooling or an explicitly retained non-editor Python
owner.

Phase 6H RefIndex parity is covered by focused Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6h_*.result.json
```

Those goldens cover a representative document open result, sidecar
`getRefIndex`, overview-only mutation, undo/redo after mutation, and a
ref-parameter fixture `getRefIndex` result. They normalize `documentId`, paths,
authored roots, and diagnostic files. The `ref_params` open-document result is
not used as a full-document golden because Python validation continues to be the
reference beyond the fixture-scoped Rust validation surface; the Phase 6H
comparison for that fixture is intentionally scoped to `getRefIndex`.

The former Phase 6H regeneration command used the removed Python editor API.
These checked-in result goldens remain parity evidence. Refresh them only after
adding Rust-native fixture tooling or an explicitly retained non-editor Python
owner.

Phase 6I non-step command RefIndex parity is covered by focused
Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6i_*.result.json
```

Those goldens intentionally compare `getRefIndex` results after input, artifact,
and artifact group mutations. They do not compare full document results for the
Phase 6I fixture because Python performs richer catalog-context validation than
the Rust backend's current fixture-scoped validation surface.

The former Phase 6I regeneration command used the removed Python editor API.
These checked-in result goldens remain parity evidence. Refresh them only after
adding Rust-native fixture tooling or an explicitly retained non-editor Python
owner.

Phase 6J.1 and Phase 6J.2 do not add Python-generated result goldens. Their Rust
tests mirror the verified Python command codec and document behavior directly,
with comments or test names covering non-obvious rules such as missing
dependency target allowance, `AddStep` empty param initialization, description
clearing, delete-step cleanup, params-only ref lifting, StepSpec default
omission, constraints application failures, and literal condition params.

## One-Shot Mode

Run one request as a single JSON argument:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"hello"}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"listStepSpecs"}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"emitRecipeYamlFromPath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"validateRecipePath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}'
```

Expected `hello` stdout is one JSON response envelope:

```json
{"ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","saveRecipeAs","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate","getRefIndex"]}}
```

`listStepSpecs` returns `{"stepSpecs":[...]}` inside the success envelope.
`emitRecipeYamlFromPath` returns `{"yaml":"..."}` inside the success envelope.
`validateRecipePath` returns `{"diagnostics":[...]}` inside the success envelope.

## JSONL Sidecar Mode

Run the sidecar loop with `--sidecar`:

```bash
printf '%s\n' '{"id":"hello-1","type":"hello","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"specs-1","type":"listStepSpecs","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"emit-1","type":"emitRecipeYamlFromPath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"validate-1","type":"validateRecipePath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Expected `hello` stdout is one JSON response line:

```json
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","saveRecipeAs","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate","getRefIndex"]}}
```

Session APIs are sidecar-only. A practical manual smoke is:

1. Start an interactive sidecar:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

2. Paste an `openRecipe` JSON line, then copy the returned `documentId` into
   `applyRecipeCommand`, `undo`, `redo`, `emitYaml`, `validate`, `getRefIndex`,
   `saveRecipe`, `getDocument`, or `closeDocument` JSON lines before sending EOF. Use a
   temporary copy of a fixture if the smoke includes `saveRecipe`:

```json
{"id":"open-1","type":"openRecipe","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}
```

The automated Rust integration tests keep the sidecar process alive and cover
`openRecipe`, `getDocument`, `applyRecipeCommand`, `undo`, `redo`, `emitYaml`,
`validate`, `getRefIndex`, `saveRecipe`, and `closeDocument` without adding any production
test-helper command.

Request-level errors are returned as API envelopes and do not terminate the
sidecar:

```bash
printf '%s\n%s\n' '{"id":"bad","type":"unknown"}' '{"id":"specs-2","type":"listStepSpecs"}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Mixed sidecar and one-shot CLI usage is a process-level usage error:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar '{"type":"hello"}'
```

That command exits non-zero, writes the usage error to stderr, and emits no API
envelope on stdout.

## Validation

Run the crate tests with:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
```

That command does not require ADB, an emulator, a physical Android device, an
APK, or any Phase 6R manual-test environment variables. Ignored real-device
tests can be listed with:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml phase6r_manual -- --ignored
```

Run a real-device test only after deliberately setting the global opt-in and any
per-test opt-in variables described in the Phase 6R section. For example, a
read-only package check can be run with:

```bash
EMUCHEF_RUN_REAL_ADB_TESTS=1 \
EMUCHEF_TEST_DEVICE_SERIAL=emulator-5554 \
EMUCHEF_TEST_PACKAGE=com.example.testapp \
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  phase6r_manual_real_adb_package_installed_check -- --ignored --nocapture
```

A device-affecting permission test requires both the global opt-in and an exact
test-package allowlist:

```bash
EMUCHEF_RUN_REAL_ADB_TESTS=1 \
EMUCHEF_RUN_REAL_ADB_PERMISSION_TEST=1 \
EMUCHEF_TEST_DEVICE_SERIAL=emulator-5554 \
EMUCHEF_TEST_PACKAGE=com.example.testapp \
EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.example.testapp \
EMUCHEF_TEST_RUNTIME_PERMISSION=android.permission.POST_NOTIFICATIONS \
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  phase6r_manual_real_adb_runtime_permission_requires_allowlist -- --ignored --nocapture
```
