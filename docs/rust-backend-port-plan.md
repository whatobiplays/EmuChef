# Rust Backend Port Assessment and Parity Plan

> Historical note: this Phase 6B plan is superseded for current runtime status by
> `docs/rust-backend-cutover-readiness.md`. Paths and bridge descriptions below
> describe the migration snapshot at planning time unless explicitly marked as
> updated. The current Tauri editor runtime launches the Rust sidecar directly
> and has no Python bridge, Python fallback, backend selector, or backend toggle.
> The Python editor API paths named below were planning-time migration sources
> and are no longer active source/test paths; those references are retained as
> historical migration evidence.

## Summary

Phase 6B was a planning-only assessment for a future Rust implementation of the
EmuChef editor backend. At planning time, the editor backend implementation was
Python and Python remained the reference implementation until a Rust backend
passed agreed parity tests.

No Rust backend crate existed in this phase. This plan did not change Tauri
backend selection, sidecar protocol behavior, Python behavior, planner behavior,
executor behavior, CLI behavior, packaging, or UI scope.

The planning-time backend truth was spread across these repo areas:

- `src/emuchef/`: authored models, loader, validation, planner, executor, step
  registry, YAML helpers, and CLI.
- `src/emuchef_editor/core/`: UI-free editor document behavior, YAML emission,
  validation adapters, ref indexing, and command application.
- `src/emuchef_editor/api/`: JSON DTOs, command codec, one-shot server,
  session manager, structured errors, protocol metadata, and JSONL sidecar.
- `apps/config-editor/src-tauri/`: historical Tauri v2 bridge and persistent
  sidecar client planning area; the current runtime bridge now launches the Rust
  sidecar directly.
- `apps/config-editor/src/`: React/TypeScript editor API types, command payload
  types, sidecar API calls, and editor UI behavior.
- `tests/` and `apps/config-editor/tests/`: current Python and frontend parity
  evidence.

At Phase 6B inspection time, `src/emuchef/steps/builtin.py` registers nine
first-party `StepPlugin(...)` entries. The registry path and plugin metadata are
the source of truth; future Rust work should not depend on a hard-coded count.

Recommended next phase: Phase 6C should create only a Rust backend skeleton for
the compatibility surface: `hello`, response envelopes/errors, the JSONL
sidecar loop, and one-shot `hello`. Phase 6C should not load recipes, create
document sessions, apply commands, edit documents, validate recipes, emit recipe
YAML, or change Tauri backend selection.

## Planning-Time Python Backend Inventory

| Subsystem | What it does today | Existing paths | Editor dependency | Planner/executor dependency | Port difficulty | Suggested Rust phase |
| --- | --- | --- | --- | --- | --- | --- |
| Authored recipe model | Defines `Recipe`, inputs, artifacts, artifact groups, steps, conditions, constraints, permission helper models, and param/ref values. | `src/emuchef/domain/recipe.py`, `src/emuchef/domain/step.py`, `src/emuchef/domain/input_declaration.py`, `src/emuchef/domain/artifacts.py`, `src/emuchef/domain/param_values.py`, `src/emuchef/domain/refs.py` | Direct through editor DTOs and commands. | Direct for loader, planner, executor. | High | 6E, then broaden through 6I |
| Authored YAML load | Uses `yaml.safe_load`, validates `schema_version` and `kind`, parses known authored sections, rejects top-level `permissions:`, preserves authored params as typed refs only for exact top-level `{ref: ...}` param values. | `src/emuchef/io/loader.py` | Direct through `load_recipe_document`. | Direct for catalog and planner inputs. | High | 6E |
| Canonical recipe YAML emit/save | Emits canonical authored recipe YAML using `yaml.safe_dump(..., sort_keys=False, allow_unicode=True)`, fixed top-level order, explicit ref objects, step-param ordering from the step registry, and no comment/format preservation. | `src/emuchef_editor/core/yaml/writer.py`, `src/emuchef_editor/core/documents/recipe_document.py` | Direct for preview, dirty state, save, Save As. | Indirect for editor-authored files later consumed by planner. | High | 6E |
| General YAML serde | Dumps dataclass-based YAML for CLI artifacts and loads YAML for execution plans and operation files. | `src/emuchef/io/serde.py` | Indirect only. | Direct for CLI/planner/executor files. | Medium | 6I or CLI phase |
| Validation | Provides single-file, in-memory recipe, and full-catalog validation. Produces warnings/errors with file, object kind/id, and field details. Performs local recipe checks plus catalog cross-reference checks when `authored_root` is available. | `src/emuchef/io/validation.py`, `src/emuchef_editor/core/validation/validator_service.py` | Direct for diagnostics in documents and path validation. | Direct for CLI validation; shares planner contract checks. | High | 6E minimal, full later |
| Planner contracts | Validates step params, ref format, unknown refs, default injection, authored-to-execution ref normalization, and plugin-owned validation/normalization hooks. | `src/emuchef/planner/contracts.py`, `src/emuchef/steps/planner_hooks.py` | Indirect through validation diagnostics. | Direct planner dependency. | High | 6I |
| Planner service and draft plan | Builds selected recipe/step draft state, expands recipe dependencies, applies operations, tracks planner-session history, prunes direct consumers of unbound optional inputs, emits execution plans. | `src/emuchef/planner/service.py`, `src/emuchef/planner/draft_builder.py`, `src/emuchef/planner/dependencies.py`, `src/emuchef/planner/emitter.py`, `src/emuchef/planner/bindings.py`, `src/emuchef/planner/conflicts.py`, `src/emuchef/planner/operations.py` | Not directly used by current Tauri editor. | Core planner behavior. | High | 6I |
| Execution plan IO | Loads execution plans and `planning_result.execution_plan`, rejects planner-only fields and unsupported step types, parses execution refs/literals/runtime values. | `src/emuchef/io/execution_plan_io.py`, `src/emuchef/domain/execution_plan.py` | No direct Tauri editor dependency. | Direct executor/CLI dependency. | Medium | 6I or 6J |
| Executor | Runs a normalized plan single-threaded, resolves runtime refs, handles skip/verify, dependency blocking, progress events, capability/conflict checks, and step dispatch. | `src/emuchef/executor/runner.py`, `src/emuchef/executor/resolver.py`, `src/emuchef/executor/result.py`, `src/emuchef/executor/conditions.py`, `src/emuchef/executor/step_runtime.py` | No current editor dependency. | Core executor behavior. | High | 6J |
| Executor device/file helpers | Owns ADB process calls, dry-run ADB, app-private copy handling, artifact download/extract, permission command helpers, and runtime value coercion. | `src/emuchef/executor/adb.py`, `src/emuchef/executor/artifact_io.py`, `src/emuchef/executor/copy_helpers.py`, `src/emuchef/executor/permission_helpers.py`, `src/emuchef/executor/runtime_values.py` | No current editor dependency. | Direct executor dependency. | High | 6J |
| Step plugin/spec system | Registers first-party built-in step plugins with specs, editor metadata, output metadata, planner hooks, and direct executor handler callables. No external plugin discovery is implemented. | `src/emuchef/steps/contracts.py`, `src/emuchef/steps/builtin.py`, `src/emuchef/steps/planner_hooks.py`, `src/emuchef/steps/handlers/*.py`, `src/emuchef/domain/step_specs.py` | Direct for `StepSpecDto`, ref filters, param order, rich param metadata. | Direct for planner validation/normalization and executor dispatch. | Medium to high | 6D for metadata, 6I/6J for hooks/handlers |
| Ref index | Builds authored ref lists and typed candidates for input refs, artifact runtime field refs, step refs, and registered step outputs. | `src/emuchef_editor/core/refs/ref_index.py`, `src/emuchef/planner/contracts.py`, `src/emuchef/steps/builtin.py` | Direct for ref picker and DTOs. | Indirect through shared ref semantics. | Medium | 6H |
| Editor document authority | Owns the in-memory typed `Recipe`, canonical YAML cache, baseline YAML, dirty state, validation result, ref index, save/save_as, and command application. | `src/emuchef_editor/core/documents/recipe_document.py` | Direct. | No planner/executor dependency except shared model/validation. | High | 6F |
| Editor commands | Dataclass command set plus `apply_recipe_command`, safe delete, supported in-file ref rewrites, default-param omission, full `UpdateStepParams` replacement, and advanced internals updates. | `src/emuchef_editor/core/documents/commands.py`, `src/emuchef_editor/core/analysis/usages.py` | Direct through sidecar commands and PySide fallback. | Indirect through authored model validity. | High | 6G |
| Undo/redo | Snapshot-based command history with full before/after `Recipe` deep copies; undo/redo persists across save and Save As. | `src/emuchef_editor/core/documents/history.py`, `src/emuchef_editor/core/documents/recipe_document.py` | Direct. | None. | Medium | 6F |
| API DTOs | Projects live documents, recipes, diagnostics, ref index, command results, and step specs into JSON-safe camelCase DTOs. | Historical: `src/emuchef_editor/api/dto.py`; current TypeScript DTOs live in `apps/config-editor/src/api/types.ts`. | Historical direct dependency; current active editor runtime is Rust. | None. | Medium | 6D, 6E, 6F |
| Command codec | Decodes external JSON command payloads into Python command dataclasses and maps invalid shapes to structured `invalid_command` errors. | Historical: `src/emuchef_editor/api/command_codec.py`; current TypeScript command shapes live in `apps/config-editor/src/api/commands.ts`. | Historical direct dependency; current active editor runtime is Rust. | None. | Medium | 6G |
| Structured API errors | Defines stable editor API error codes and JSON-safe error details. | Historical: `src/emuchef_editor/api/errors.py`. | Historical direct dependency; current active editor runtime is Rust. | None. | Low | 6C |
| Protocol metadata | Defines protocol version 1 and required/optional/reported backend-agnostic capabilities. | Historical: `src/emuchef_editor/api/protocol.py`. | Historical direct dependency; current active editor runtime is Rust. | None. | Low | 6C |
| One-shot API server | Handles stateless `hello`, `listStepSpecs`, `openRecipe`, `validateRecipePath`, and `emitRecipeYamlFromPath` requests. | Historical: `src/emuchef_editor/api/server.py`; `python_bridge.rs` is superseded and removed from the current Tauri runtime. | Removed from active source/test paths after Rust protocol coverage. | None. | Low to medium | 6C to 6E |
| JSONL sidecar | Handles persistent request sessions over stdin/stdout JSON Lines, one request per line, one response per line, stderr for diagnostics, EOF clean exit, and session-backed document operations. | Historical: `src/emuchef_editor/api/sidecar.py`, `src/emuchef_editor/api/session.py`; current Tauri `sidecar_client.rs` launches the Rust sidecar instead. | Historical Python backend; no longer direct current Tauri backend. | None. | Medium | 6C, 6F, 6G |
| Tauri Rust bridge | Historical planning row. The current bridge starts `emuchef-rust-backend --sidecar`, sends `hello`, gates compatibility on protocol version and required capabilities, serializes one JSONL request at a time, and exposes Tauri invoke commands. | `apps/config-editor/src-tauri/src/sidecar_client.rs`, `apps/config-editor/src-tauri/src/commands.rs`, `apps/config-editor/src-tauri/src/lib.rs` | Direct current Tauri runtime, Rust only. | None. | Medium | 6K only after Rust parity |
| Tauri frontend API | Defines TypeScript DTOs, command union, sidecar API calls, transport/API error split, action gating, and command-in-flight behavior. | `apps/config-editor/src/api/types.ts`, `apps/config-editor/src/api/commands.ts`, `apps/config-editor/src/api/editorApi.ts`, `apps/config-editor/src/components/phase5EditorState.logic.ts` | Direct. | None. | Medium if changed, but should not change early. | 6K |
| CLI | Supports `draft`, `plan`, `apply`, `detect`, `detect-profiles`, and `validate`; uses Python loader/planner/executor/serde. | `src/emuchef/cli.py`, `pyproject.toml` | No direct Tauri editor dependency. | Direct top-level runtime surface. | High | After 6I/6J |
| Templates | Provides authored YAML examples and PySide creation sources; not loaded by CLI as authored inputs. | `templates/authored/`, `tests/test_templates.py` | PySide fallback and template creation API depend on recipe templates. | No direct planner dependency unless copied into `authored/`. | Low to medium | Defer unless template creation is included |
| Tests and fixtures | Define current behavior via unit tests, helper authored trees, real authored sample files, Tauri Rust unit tests, and frontend logic tests. | `tests/`, `tests/support.py`, `authored/`, `templates/authored/`, `apps/config-editor/tests/`, `apps/config-editor/src-tauri/src/*` test modules | Direct parity evidence. | Direct planner/executor/CLI evidence. | Medium | Start harness in 6C/6D, expand every phase |

## Planning-Time Protocol and API Contract

The future Rust backend must satisfy the current backend-facing contract before
it can replace Python.

The first dual-backend parity fixture must be `hello` because Tauri uses it as
the compatibility gate before normal document requests.

### Hello and Capabilities

Historical/planning-time sources:

- `src/emuchef_editor/api/protocol.py`
- `src/emuchef_editor/api/server.py`
- `src/emuchef_editor/api/sidecar.py`
- `apps/config-editor/src-tauri/src/sidecar_client.rs`
- `tests/test_editor_api_server.py`
- `tests/test_editor_api_sidecar.py`

Current `hello` behavior:

- Request type is `hello`.
- One-shot request shape is `{"type":"hello","payload":{}}`; omitted payload is accepted.
- Sidecar request shape is `{"id":"...","type":"hello","payload":{}}`; omitted payload is accepted by current sidecar request handling because missing payload becomes `{}`.
- Unknown object payload keys are ignored.
- Non-object payload returns `invalid_request`.
- Success result contains `protocolVersion: 1`.
- Success result contains a string `capabilities` list.
- The result must not expose `implementation` or `implementationVersion`.
- There is no protocol negotiation in the current contract.

Required capabilities:

- `listStepSpecs`
- `openRecipe`
- `getDocument`
- `applyRecipeCommand`
- `undo`
- `redo`
- `saveRecipe`
- `validate`
- `emitYaml`
- `getRefIndex`

Optional capabilities reported by the historical Python editor API:

- `createRecipeFromTemplate`
- `closeDocument`
- `saveRecipeAs`

Tauri compatibility behavior:

- `apps/config-editor/src-tauri/src/sidecar_client.rs` starts the sidecar on the first real sidecar request.
- It sends `hello` first, requires `protocolVersion == 1`, and requires all required capabilities.
- Extra capabilities are accepted.
- Missing required capabilities mark the sidecar incompatible.
- `sidecar_status` reports cached compatibility state and does not start the sidecar.

### Envelopes and Errors

Historical/planning-time sources:

- `src/emuchef_editor/api/protocol.py`
- `src/emuchef_editor/api/errors.py`
- `apps/config-editor/src/api/types.ts`
- `apps/config-editor/src/api/editorApi.ts`

API envelopes:

- Success: `{"ok": true, "result": {...}}`
- Failure: `{"ok": false, "error": {"code": "...", "message": "...", "details": {...}}}`
- Sidecar responses include an outer `id`: `{"id": "...", "ok": true, "result": {...}}`.
- Malformed JSONL lines return `id: null` with `ok: false`.
- Debug details are opt-in through request `debug: true` and are diagnostics, not behavior contracts.

Structured editor API error codes:

- `unknown_document`
- `invalid_request`
- `invalid_command`
- `command_failed`
- `load_failed`
- `save_failed`
- `validation_failed`
- `internal_error`

Tauri treats API `ok:false` envelopes as successful transport responses. It
treats malformed envelopes, missing ids, mismatched ids, invalid stdout JSON, or
process failures as transport errors.

### Request Surface

Historical one-shot stateless requests in `src/emuchef_editor/api/server.py`:

- `hello`
- `listStepSpecs`
- `openRecipe`
- `validateRecipePath`
- `emitRecipeYamlFromPath`

Historical persistent sidecar requests in `src/emuchef_editor/api/sidecar.py`:

- `hello`
- `listStepSpecs`
- `openRecipe`
- `createRecipeFromTemplate`
- `closeDocument`
- `getDocument`
- `saveRecipe`
- `saveRecipeAs`
- `emitYaml`
- `applyRecipeCommand`
- `undo`
- `redo`
- `validate`
- `getRefIndex`

Current Tauri invoke names in `apps/config-editor/src-tauri/src/commands.rs`:

- `list_step_specs`
- `open_recipe`
- `validate_recipe_path`
- `emit_recipe_yaml_from_path`
- `sidecar_status`
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

The Rust backend should implement the backend request types above. It should not
change Tauri invoke names or frontend DTO types during early port phases.

### DTO Surface

Historical/current DTO evidence sources:

- `src/emuchef_editor/api/dto.py`
- `apps/config-editor/src/api/types.ts`
- `tests/test_editor_api_dto.py`
- `tests/test_editor_api_step_specs.py`

High-level DTOs:

- `RecipeDocumentDto`: `documentId`, `path`, `authoredRoot`, `dirty`,
  `canUndo`, `canRedo`, `recipe`, `yaml`, `diagnostics`, `refIndex`.
- `RecipeDto`: `schemaVersion`, `kind`, `id`, `name`, `description`,
  `recipeDependencies`, `provides`, `inputs`, `artifacts`, `artifactGroups`,
  `steps`.
- `StepDto`: `id`, `type`, `name`, `description`, `userToggleable`,
  `dependencies`, `constraints`, `skipIf`, `params`, `verify`.
- `DiagnosticDto`: `severity`, `code`, `message`, `file`, `objectKind`,
  `objectId`, `field`.
- `RefIndexDto`: `inputRefs`, `artifactRefs`, `stepRefs`, `stepOutputRefs`,
  `allRefs`, `candidates`.
- `StepSpecDto`: `type`, `label`, `supported`, `primaryOutputName`,
  `outputs`, `paramOrder`, `params`, `defaults`, `refFilters`.
- `CommandResultDto`: `changed`.

DTO requirements:

- DTOs must be JSON-safe primitives, arrays, and objects.
- DTO keys are camelCase.
- Authored YAML keys remain snake_case.
- `RefParamValue` is projected as `{"ref": "..."}`.
- `RecipeDocumentDto` does not include step specs.
- Top-level recipe `permissions` is absent from DTOs because that authored shape is invalid.

### Sidecar Lifecycle

Historical/planning-time sources:

- `src/emuchef_editor/api/sidecar.py`
- `apps/config-editor/src-tauri/src/sidecar_client.rs`
- `tests/test_editor_api_sidecar.py`

Lifecycle requirements:

- Sidecar reads UTF-8 JSON Lines from stdin.
- Sidecar writes exactly one UTF-8 JSON response line to stdout per request.
- Stdout is machine-readable JSONL only.
- Diagnostics and logs belong on stderr.
- Valid sidecar requests require a non-empty string `id`.
- Each response echoes the id.
- Malformed JSON lines return `id: null`.
- API request failures do not terminate the process.
- Sidecar exits cleanly when stdin reaches EOF.
- Tauri serializes sidecar requests one send-line/read-line exchange at a time.
- Tauri does not automatically restart an exited or incompatible sidecar.

### Unknown Document Behavior

Historical/planning-time sources:

- `src/emuchef_editor/api/session.py`
- `tests/test_editor_api_session.py`
- `tests/test_editor_api_sidecar.py`
- `apps/config-editor/src/components/phase5EditorState.logic.ts`

Unknown document behavior:

- Missing session ids return `ok:false`.
- Error code is `unknown_document`.
- Details include `documentId`.
- Tauri invalidates the current document session only when `unknown_document`
  applies to the current document id.

## Target Rust Backend Architecture

### Recommendation

Start with a Rust external JSONL sidecar implementing the same backend-agnostic
protocol. Do not move backend behavior in-process into Tauri until parity is
proven.

Reasons:

- Reuses the existing `hello`/capabilities gate.
- Lets Python and Rust run side by side under the same parity harness.
- Avoids early Tauri frontend changes.
- Preserves crash isolation.
- Keeps current Tauri bridge semantics stable while the backend changes behind
  the protocol.
- Makes it possible to compare complete JSON envelopes before any cutover.

### Proposed Future Paths

The following paths are proposed future paths. They do not exist in Phase 6B.

Conservative single-crate start:

```text
crates/emuchef-rust-backend/
  Cargo.toml
  src/
    main.rs
    lib.rs
    protocol/
    dto/
    model/
    yaml/
    validation/
    step_specs/
    refs/
    session/
    commands/
    jsonl_sidecar.rs
    one_shot.rs
```

This single-crate start is lower risk for Phase 6C through 6H because the first
goal is editor protocol parity, not reusable Rust product architecture.

Eventual workspace split, only after editor parity is stable:

```text
crates/emuchef-core/
  src/
    lib.rs
    recipe/
    yaml/
    validation/
    step_specs/
    refs/
    planner/
    executor/
crates/emuchef-editor-backend/
  src/
    lib.rs
    protocol/
    dto/
    session/
    commands/
    jsonl_sidecar.rs
    one_shot.rs
crates/emuchef-cli/
  src/
    main.rs
```

Split when planner or CLI reuse becomes real, or when the editor backend crate
starts carrying unrelated executor/device dependencies.

### Tauri Integration Options

Option A: external Rust sidecar implementing JSONL.

- Pros: protocol reuse, parity harness friendliness, crash isolation, minimal UI
  disruption.
- Cons: separate process packaging later, startup/process lifecycle work.
- Fit: best initial target.

Option B: in-process Rust Tauri commands.

- Pros: fewer processes, simpler long-term packaging if Tauri remains the only
  client.
- Cons: bypasses the current sidecar protocol, makes dual-backend parity harder,
  couples backend lifecycle to Tauri, reduces crash isolation.
- Fit: not appropriate before parity.

Option C: hybrid sidecar first, in-process later.

- Pros: proves parity using current protocol, keeps the option to collapse into
  Tauri later.
- Cons: may require a second integration pass.
- Fit: recommended roadmap.

Recommended target: hybrid, with Rust JSONL sidecar first.

## Data Model Mapping

### Mapping Table

| Python concept | Existing paths | Rust representation target | Notes |
| --- | --- | --- | --- |
| `Recipe` | `src/emuchef/domain/recipe.py` | Strong `serde` struct with `IndexMap` for ordered mappings | DTO layer camelCase; YAML layer snake_case. |
| `InputDeclaration` and `InputValidation` | `src/emuchef/domain/input_declaration.py` | Strong structs/enums | Preserve default/null semantics. |
| `RemoteFileArtifact` | `src/emuchef/domain/artifacts.py` | Strong enum/struct | Current artifact kind support is `remote_file`. |
| Artifact groups | `src/emuchef/domain/recipe.py` | `IndexMap<String, Vec<String>>` | Preserve authored/editor-managed order. |
| `Step` | `src/emuchef/domain/step.py` | Strong struct plus flexible params map | Step type remains registry-owned string. |
| Step params | `src/emuchef/domain/param_values.py`, `src/emuchef/io/loader.py` | `IndexMap<String, AuthoredParamValue>` where `AuthoredParamValue` is `Ref` or JSON literal | Only top-level exact `{ref: string}` values become refs. |
| Constraints | `src/emuchef/domain/step.py` | Strong struct | YAML uses `conflicts_with`; DTO/command uses `conflictsWith`. |
| `skip_if` and `verify` | `src/emuchef/domain/step.py` | Vec of condition structs with `serde_json::Value` params | Nested `{ref: ...}` remains literal JSON. |
| Grant permissions | `src/emuchef/domain/recipe.py`, `src/emuchef/steps/planner_hooks.py`, `src/emuchef/executor/permission_helpers.py` | Structured value helpers over JSON params | Current authored data is step-local under `grant_permissions.params.runtime`, `appops`, `policy`. |
| Diagnostics | `src/emuchef_editor/core/validation/validator_service.py` | Strong DTO structs | Preserve severity/code/message/file/objectKind/objectId/field. |
| Ref index | `src/emuchef_editor/core/refs/ref_index.py` | Strong structs | Preserve all ref lists and candidate value types. |
| Step specs | `src/emuchef/domain/step_specs.py`, `src/emuchef/steps/contracts.py`, `src/emuchef/steps/builtin.py` | Static Rust registry structs first | External plugin loading remains deferred. |
| Commands | `src/emuchef_editor/core/documents/commands.py`, `src/emuchef_editor/api/command_codec.py` | Tagged command enum plus validation decoder | Match current command payload shapes exactly. |

### Structs vs Flexible Values

Use strong `serde` structs where Python has stable domain models and command
contracts. Use `serde_json::Value` or equivalent for:

- Step params that are not top-level refs.
- Condition params in `skip_if` and `verify`.
- Input `default`.
- Input `metadata`.
- Device plan `defaults`, `overrides`, and metadata in later planner phases.

Use `indexmap::IndexMap` for mappings whose order is visible in DTOs or YAML:

- `inputs`
- `artifacts`
- `artifact_groups`
- step `params`
- condition `params`
- metadata maps where canonical output matters

Use optional fields to distinguish omitted vs null only where Python behavior
does. For command payloads, preserve current field requirements from
`src/emuchef_editor/api/command_codec.py`.

### Unknown Fields

Current behavior is mixed:

- Top-level recipe `permissions` is explicitly rejected in `src/emuchef/io/loader.py`
  and `src/emuchef/io/validation.py`.
- Unsupported or extra step params are preserved by the editor model and
  canonical YAML when unrelated supported fields are edited, but validation can
  report step contract errors.
- Unsupported constraints, `skip_if`, and `verify` entries are preserved
  semantically when edited outside those fields.
- Unknown or extra keys inside structured object params can be preserved by the
  frontend and backend when the authored value can be copied without data loss.
- The canonical recipe writer only emits known top-level recipe fields; broad
  preservation of unknown top-level recipe fields is not established as a
  current contract.

Open question: whether a Rust backend should intentionally preserve unknown
top-level fields that Python currently drops during canonical emission. The
default parity rule should be to match Python, not to invent broader
preservation.

## YAML Load and Emit Strategy

### Current Semantics

Historical/planning-time sources:

- `src/emuchef/io/loader.py`
- `src/emuchef/io/serde.py`
- `src/emuchef_editor/core/yaml/loader.py`
- `src/emuchef_editor/core/yaml/writer.py`
- `tests/test_editor_core.py`
- `tests/test_step_plugins.py`
- `CONTEXT.md`

Current YAML loading:

- Uses `yaml.safe_load`.
- Empty YAML loads as `{}` at the low-level loader.
- Top-level content must be a mapping.
- `schema_version: 1` and expected `kind` are required for loader success.
- Authored recipes use maps for `inputs`, `artifacts`, and `artifact_groups`.
- Authored refs are explicit `{ref: "..."}` only.
- Top-level `permissions:` is invalid.
- Step type ids are strings. Single-recipe editor loads reject unsupported step
  types against the built-in registry; full-catalog validation also applies
  registry-backed contract checks.

Current recipe YAML emission:

- Uses `yaml.safe_dump(build_recipe_payload(recipe), sort_keys=False, allow_unicode=True)`.
- Does not preserve comments or arbitrary formatting.
- Emits canonical top-level order:
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
- Omits `description` when it is `None`.
- Preserves insertion/editor order for input, artifact, artifact-group, and
  membership maps/lists.
- Sorts generic JSON mapping keys inside step params and condition params.
- Orders step params by `StepEditorMetadata.param_order`, then remaining spec
  params, then sorted extra params.
- Omits step params that equal registry defaults during command application.
- Emits `conflicts_with` in YAML and `conflictsWith` in DTO/command payloads.

Validation relationship:

- Loading can fail before validation for unsupported schema/kind/shape.
- Editor validation refreshes after document construction and after command
  application.
- Save writes canonical YAML and updates the saved baseline.

### Rust YAML Library Options

`serde_yaml`:

- Use for first Rust backend because it integrates with `serde` structs.
- Pair with `indexmap` support for order-sensitive mappings.
- Risk: emitter formatting differs from PyYAML. Golden emitted YAML tests must
  decide whether byte-for-byte equality or semantic equality is required.
- Do not lock this choice until golden canonical-emission tests prove the output
  parity is acceptable against Python.

`serde_yml`:

- Consider if project policy prefers maintained alternatives to `serde_yaml`.
- Same order/formatting risks apply.

`yaml-rust` or `yaml-rust2`:

- Consider if lower-level control or custom preserving behavior becomes
  necessary.
- More manual mapping to domain structs.

Custom emitter:

- Use only if canonical output must match Python byte-for-byte and serde-based
  emission cannot be made stable enough.
- Higher maintenance cost.

Conservative first approach:

- Parse into an order-preserving intermediate YAML/JSON representation.
- Convert into strong Rust domain structs.
- Emit canonical YAML from a deliberate ordered payload builder that mirrors
  `src/emuchef_editor/core/yaml/writer.py`.
- Use golden tests for emitted YAML and semantic round-trips before dogfooding.
- For editor document parity, initially target byte-for-byte canonical YAML
  equality with Python because `RecipeDocument.apply_command` and dirty state
  depend on canonical emitted YAML comparison, not just parsed semantic
  equivalence.

Primary YAML risks:

- Comments and hand formatting are not preserved today, but users may still
  notice formatting drift if Rust emits differently.
- Mapping order drift can change dirty state and diffs.
- Null vs omitted values can change authored semantics.
- Enum/string representation must remain plain string values.
- Unknown top-level field preservation is unclear and should be treated as an
  open question.

## Command, Session, and Undo/Redo Strategy

Historical/planning-time sources:

- `src/emuchef_editor/core/documents/recipe_document.py`
- `src/emuchef_editor/core/documents/commands.py`
- `src/emuchef_editor/core/documents/history.py`
- `src/emuchef_editor/api/command_codec.py`
- `src/emuchef_editor/api/session.py`
- `tests/test_editor_core.py`
- `tests/test_editor_api_command_codec.py`
- `tests/test_editor_api_session.py`

Current mutation model:

- `RecipeDocument` is the document authority.
- Commands operate on the typed `Recipe`, not raw YAML.
- `apply_command` returns `changed: false` when the emitted canonical YAML is
  unchanged.
- No-op commands are not recorded in undo history.
- Undo/redo stores full before/after recipe snapshots.
- Dirty state is canonical YAML compared with the saved baseline.
- Save updates the baseline and keeps undo/redo history.
- Save As writes canonical YAML to a new path, updates document path and
  baseline, and keeps undo/redo history.
- `UpdateStepParams` replaces the entire params object for the selected step.
- Selecting refs does not synthesize dependencies.
- Safe delete removes supported structured references, dependencies, conflicts,
  artifact-group memberships, and supported selection-list entries.
- Unsupported step content is preserved when supported surfaces are edited.

Rust equivalents:

- `DocumentSessionManager` owns `HashMap` or `IndexMap` of `document_id` to
  live session documents.
- `RecipeDocument` struct owns path, authored root, working recipe, canonical
  YAML, baseline YAML, validation result, ref index, and history.
- Command decoder maps external JSON into a tagged `Command` enum with exact
  current payload validation.
- Command applier returns `(updated_recipe, operation_label)`.
- Dirty/no-op detection compares canonical emitted YAML, not structural equality.
  The initial Rust editor parity target should therefore be byte-for-byte
  canonical YAML equality with Python for all document and command fixtures.
- Undo/redo should initially use full recipe snapshots to match Python behavior.
- Error mapping should preserve `invalid_command`, `command_failed`,
  `unknown_document`, `save_failed`, and `validation_failed`.

Highest-risk commands:

- `DeleteInput`, `DeleteArtifact`, `DeleteArtifactGroup`, and `DeleteStep`
  because they perform safe cleanup across supported references.
- `RenameInput`, `RenameArtifact`, `RenameArtifactGroup`, `RenameStep`, and
  `RenameRecipeId` because they rewrite only supported structured usages and
  must preserve unsupported content.
- `UpdateStepParams` because it is a full replacement and only top-level exact
  ref objects are refs.
- `UpdateStepConstraints` because command keys use `conflictsWith` while YAML
  uses `conflicts_with`.
- `UpdateStepSkipIf` and `UpdateStepVerify` because nested refs remain literals.

## Validation Parity Strategy

Historical/planning-time sources:

- `src/emuchef/io/validation.py`
- `src/emuchef/planner/contracts.py`
- `src/emuchef/steps/planner_hooks.py`
- `src/emuchef_editor/core/validation/validator_service.py`
- `tests/test_validation.py`
- `tests/test_editor_core.py`

Current validation surface:

- `validate_authored_catalog(root)` validates the full authored catalog.
- `validate_authored_path(path, authored_root)` validates one file and optional
  catalog context.
- `validate_authored_recipe(recipe, path, authored_root)` validates an unsaved
  in-memory recipe and can replace the current on-disk contribution by path for
  catalog-context checks.
- Without `authored_root`, single recipe validation returns a
  `validation_context_limited` warning.
- Diagnostics expose severity, code, message, file, object kind, object id, and
  field.
- Step contract diagnostics come from the planner/step registry layer.

Rust port strategy:

- First implement editor-minimal validation: local recipe shape, top-level
  permissions rejection, supported step types, required params, ref format, and
  current diagnostic DTO shape.
- Then add catalog-context replacement behavior and cross-file checks.
- Full CLI catalog validation comes later with planner parity.
- Use golden diagnostics fixtures keyed by authored YAML input and expected
  diagnostic DTOs.
- Compare both warning and error ordering before replacement.

Compatibility expectation:

- Python diagnostic output is authoritative until Rust golden diagnostics match
  for the agreed matrix.

## Ref Index Parity Strategy

Historical/planning-time sources:

- `src/emuchef_editor/core/refs/ref_index.py`
- `src/emuchef/planner/contracts.py`
- `src/emuchef/steps/builtin.py`
- `apps/config-editor/src/components/stepParams.logic.ts`
- `tests/test_editor_core.py`

Current ref index behavior:

- `inputRefs`: `inputs.<id>` for sorted input ids.
- `artifactRefs`: `artifacts.<id>.<field>` for sorted artifact ids and sorted
  runtime artifact fields from `RUNTIME_ARTIFACT_FIELDS`.
- `stepRefs`: `steps.<id>` in authored step order.
- `stepOutputRefs`: `steps.<id>.outputs.<output>` in authored step order and
  registry output order.
- `allRefs` concatenates input, artifact, step, and step-output refs.
- `candidates` include typed input refs, artifact field refs with known runtime
  value types, and step output refs with registry output metadata.
- Missing current refs are preserved in the frontend picker by TypeScript logic,
  not generated by the backend `refIndex`.

Rust strategy:

- Port after DTO/schema and basic YAML load, before broad command parity.
- Compare complete `RefIndexDto`, including ordering.
- Keep registry output metadata as the source for step output refs.
- Add fixtures for inputs, artifacts, step primary outputs, missing refs, and
  incompatible current refs as frontend behavior.

## Step Spec and Schema Strategy

Historical/planning-time sources:

- `src/emuchef/domain/step_specs.py`
- `src/emuchef/steps/contracts.py`
- `src/emuchef/steps/builtin.py`
- `src/emuchef/steps/planner_hooks.py`
- `src/emuchef_editor/api/dto.py`
- `tests/test_editor_api_step_specs.py`
- `tests/test_step_plugins.py`

Current behavior:

- Built-in registry is the canonical source for supported step types, specs,
  planner hooks, executor handlers, editor labels, param order, ref filters,
  outputs, and primary outputs.
- `STEP_SPECS` and `PRIMARY_OUTPUT_STEP_TYPES` are compatibility projections.
- `StepSpec.executor_handler` is transitional metadata only.
- Runtime dispatch uses `StepPlugin.handler`.
- External plugin discovery is not implemented.
- `StepSpecDto` includes editor-safe metadata only; it is not mutation
  authority.
- Rich param shapes currently cover artifact id lists, artifact-group id lists,
  runtime permission rows, app-op permission rows, and permission policy fields.

Rust strategy:

- Start with a static built-in registry that exactly matches current
  `listStepSpecs` output.
- Model `ParamSpec`, `ParamShapeSpec`, `ParamFieldSpec`, outputs, editor
  metadata, defaults, enum values, and ref filters.
- Do not implement dynamic plugins early.
- Keep planner hooks and executor handlers out of early DTO-only spec parity
  until planner/executor phases.
- Golden-test full `StepSpecDto` output before using Rust in the editor.

## Planner Migration Strategy

Historical/planning-time sources:

- `src/emuchef/planner/service.py`
- `src/emuchef/planner/draft_builder.py`
- `src/emuchef/planner/dependencies.py`
- `src/emuchef/planner/emitter.py`
- `src/emuchef/planner/contracts.py`
- `src/emuchef/planner/bindings.py`
- `src/emuchef/planner/conflicts.py`
- `src/emuchef/planner/profile_matching.py`
- `tests/test_planner_core.py`
- `tests/test_cli.py`

Planner responsibilities:

- Load full authored catalog.
- Expand recipe dependencies.
- Apply device plan defaults and validated planner overrides.
- Track draft sessions and undo/redo for planner operations.
- Shape recipe/step availability by runtime capabilities and conflicts.
- Preserve authored visual order in draft state.
- Emit execution plan steps in topological execution order.
- Resolve input bindings and prune direct consumers of unbound optional inputs.
- Normalize authored refs into execution-plan refs.
- Expand artifact-group selections.
- Inject default params for execution.

Recommendation:

- Port planner after editor load/save/validation/ref/session/command parity.
- Build golden draft-plan and execution-plan fixtures before porting.
- Use current Python planner as the source for order, diagnostics, and
  normalization.
- Do not port executor before planner parity, because executor consumes
  normalized execution plans.

## Executor Migration Strategy

Historical/planning-time sources:

- `src/emuchef/executor/runner.py`
- `src/emuchef/executor/step_runtime.py`
- `src/emuchef/executor/adb.py`
- `src/emuchef/executor/artifact_io.py`
- `src/emuchef/executor/copy_helpers.py`
- `src/emuchef/executor/permission_helpers.py`
- `src/emuchef/steps/handlers/*.py`
- `tests/test_executor_core.py`
- `tests/test_cli.py`

Executor responsibilities:

- Execute normalized plans single-threaded.
- Resolve runtime refs from inputs, artifacts, and prior step outputs.
- Evaluate dependency blocking.
- Evaluate capabilities, conflicts, `skip_if`, and `verify`.
- Dispatch through registry plugin handlers.
- Download remote artifacts with strict TLS.
- Extract archives.
- Copy host/device paths, including app-private privileged staging/copy paths.
- Install APKs, launch apps, force stop apps, wait, and grant permissions.
- Support dry-run behavior through `DryRunAdb`.
- Emit progress and execution summaries.

Recommendation:

- Defer executor port until after planner parity.
- Treat executor as high risk because it crosses filesystem, network, ADB,
  shell quoting, root/device capabilities, and platform boundaries.
- Start executor parity with dry-run and mocked ADB tests before real-device
  behavior.
- Preserve the current step-local grant-permissions model and `permission_results`
  output shape.

## CLI Migration Considerations

Historical/planning-time sources:

- `src/emuchef/cli.py`
- `pyproject.toml`
- `tests/test_cli.py`

Current commands:

- `draft`
- `plan`
- `apply`
- `detect`
- `detect-profiles`
- `validate`

Recommendation:

- Keep Python CLI during editor backend porting.
- Add Rust CLI only after Rust core has planner/executor parity.
- Reuse Rust core for a future CLI, rather than building a separate CLI-only
  implementation.
- Treat CLI tests as parity tests for validation, planner, execution summary,
  ADB resolution, and dry-run behavior once Rust reaches those phases.

## Rust Crate and Dependency Recommendations

No dependencies are added in Phase 6B. Recommendations are provisional.

| Crate | Use | Rationale | Risks/alternatives |
| --- | --- | --- | --- |
| `serde` | DTOs, YAML/JSON model structs | Standard Rust serialization layer. | Must control rename rules carefully for camelCase DTOs and snake_case YAML. |
| `serde_json` | API envelopes, command payloads, flexible authored values | Current Tauri crate already uses it with preserve-order feature. | Preserve-order behavior should be explicit in backend crate. |
| `indexmap` | Ordered authored maps and JSON/YAML payload builders | Required for visible order parity. | Standard maps can cause output drift. |
| `serde_yaml` | Candidate initial YAML load/emit | Integrates with serde and is fast to prototype. | Do not lock this choice until golden canonical-emission tests prove acceptable parity with Python; consider `serde_yml` or custom emitter if formatting/order drift is unacceptable. |
| `serde_yml` | YAML alternative | Possible maintained alternative depending on project preference. | Needs evaluation against PyYAML golden output. |
| `yaml-rust2` | Lower-level YAML option | More control if canonical emitter becomes difficult. | More manual conversion code. |
| `thiserror` | Structured internal errors | Good for stable error variants. | `anyhow` is easier for prototypes but less contract-oriented. |
| `anyhow` | Early app-level error context | Useful in sidecar main/IO layers. | Do not leak arbitrary anyhow strings into stable API codes without mapping. |
| `uuid` | Document ids and request-independent ids | Python uses UUIDs for document ids. | Parity harness must normalize document ids. |
| `camino` | UTF-8 paths | Cleaner cross-platform path DTO handling. | Standard `PathBuf` may be enough at first. |
| `clap` | Future one-shot/sidecar/CLI args | Useful for sidecar and eventual CLI. | Not needed for the smallest manual skeleton. |
| `tokio` | Async sidecar/process work | Only needed if concurrent IO becomes useful. | Current protocol is serial; standard blocking IO is simpler and more faithful. |
| `tempfile` | Tests | Mirrors Python temporary fixture behavior. | Standard temp dirs can work but are clunkier. |
| `assert_cmd` | CLI/sidecar process tests | Useful for one-shot and JSONL sidecar tests. | Direct process spawning is enough early. |
| `insta` | Snapshot/golden parity tests | Useful for DTO/diagnostics/YAML diffs. | Snapshot churn can hide behavior changes if not reviewed carefully. |

## Minimum Viable Rust Backend for Dogfooding

Because current Tauri compatibility gating requires all required capabilities,
a Rust backend that is only read-only cannot be dogfooded through the existing
Tauri app without a later backend selection or read-only compatibility mode.
Phase 6B does not add such a mode.

Minimum harness dogfood:

- `hello`
- protocol version 1
- required/optional capabilities fixture
- JSONL loop
- envelopes/errors
- `listStepSpecs`
- read-only `openRecipe`, `getDocument`, `emitYaml`, `validate`, `getRefIndex`

Minimum current-Tauri dogfood:

- All harness dogfood capabilities.
- `applyRecipeCommand`
- `undo`
- `redo`
- `saveRecipe`
- Accurate `changed`, `dirty`, `canUndo`, `canRedo`, diagnostics, YAML, and
  refIndex after every mutation.
- Safe testing on temporary recipe copies only.

Simple first useful edit:

- `SetOverviewField` for `name` and `description`, with canonical YAML no-op
  detection and snapshot undo/redo.

Parity tests required before current-Tauri dogfooding:

- `hello` compatibility fixture.
- `listStepSpecs` golden output.
- Open a representative recipe and compare `RecipeDocumentDto`.
- Emit canonical YAML and compare to Python for that fixture.
- Validate and compare diagnostics.
- Compare `refIndex`.
- Apply `SetOverviewField`, undo, redo, save on a temp copy, and compare
  returned document DTOs.

## Dual-Backend Parity Harness Recommendation

Implement a future parity harness before broad Rust implementation. Suggested
future location: `tests/parity/` for Python-driven orchestration, or a proposed
Rust integration test area under `crates/emuchef-rust-backend/tests/` once that
crate exists.

Historical harness requirements:

- Start the source-checkout Python sidecar with
  `PYTHONPATH=src python -m emuchef_editor.api.server --sidecar`.
- Start the Rust sidecar through the proposed Rust binary.
- Send identical JSONL requests to each backend.
- Compare complete JSON envelopes where ids are deterministic.
- Normalize nondeterministic fields such as sidecar request ids, document ids,
  UUIDs, absolute temp paths where appropriate, and path separators if needed.
- Compare DTOs, emitted YAML, diagnostics, refIndex, StepSpecDto, command
  results, and planner output.
- Report structural diffs clearly.
- At planning time, keep Python as the oracle until replacement is explicitly
  approved. Current runtime evidence now lives in the Rust backend tests and
  no-Python-editor-API guard.

First parity fixture:

1. Send `{"id":"hello-1","type":"hello","payload":{}}`.
2. Assert both backends return `id: "hello-1"`, `ok: true`,
   `result.protocolVersion: 1`, all required capabilities, optional capability
   handling as agreed, and no implementation identity fields.

## Parity Test Matrix

| Subsystem | Historical Python tests | Fixture/golden data needed | Rust parity test type | Priority | Notes |
| --- | --- | --- | --- | --- | --- |
| Hello/protocol | `tests/test_editor_api_server.py`, `tests/test_editor_api_sidecar.py`, Rust tests in `apps/config-editor/src-tauri/src/sidecar_client.rs` | `hello` request and expected capabilities | Dual-backend JSONL fixture | P0 | Must be first because Tauri gates all later requests on it. |
| Envelopes/errors | `tests/test_editor_api_server.py`, `tests/test_editor_api_sidecar.py`, `tests/test_editor_api_session.py` | invalid JSON, unknown request, invalid command, unknown document | Dual-backend envelope comparison | P0 | API failures are not transport failures. |
| DTO serialization | `tests/test_editor_api_dto.py`, `apps/config-editor/src/api/types.ts` | representative document with diagnostics and refs | JSON DTO golden | P0 | Include JSON-safe primitive checks. |
| Step specs | `tests/test_editor_api_step_specs.py`, `tests/test_step_plugins.py` | full `listStepSpecs` output | Snapshot/golden DTO | P0 | Include param shapes, defaults, outputs, ref filters. |
| YAML load/emit | `tests/test_editor_core.py`, `tests/test_step_plugins.py`, `tests/test_templates.py` | authored recipe fixtures and expected canonical YAML | Golden YAML and semantic round-trip | P0 | Include comments/format drift note; comments are not preserved today. |
| Validation | `tests/test_validation.py`, `tests/test_editor_core.py` | expected diagnostics for local and catalog-context validation | Golden diagnostics DTO | P0 | Include limited-context warning. |
| Ref index | `tests/test_editor_core.py`, frontend ref picker tests in `apps/config-editor/tests/stepParams.logic.test.ts` | recipe with inputs, artifacts, step outputs | Golden `RefIndexDto` | P0 | Frontend missing-current-ref behavior remains frontend-owned. |
| Command codec | `tests/test_editor_api_command_codec.py`, `apps/config-editor/src/api/commands.ts` | command payloads and invalid payloads | Decoder unit tests | P0 | Preserve top-level-only ref decoding. |
| Session lifecycle | `tests/test_editor_api_session.py`, `tests/test_editor_api_sidecar.py` | open/get/close/unknown document sequence | Dual-backend JSONL sequence | P0 | Normalize document ids. |
| Undo/redo | `tests/test_editor_core.py`, `tests/test_editor_api_session.py` | command sequence with no-op and changed edits | Dual-backend command sequence | P0 | Snapshot behavior should match. |
| Save/save_as | `tests/test_editor_core.py`, `tests/test_editor_api_session.py`, `tests/test_editor_api_sidecar.py` | temp recipe copies | Filesystem integration test | P1 | Tauri UI exposes Save, not Save As. |
| Safe delete | `tests/test_editor_core.py`, `tests/test_editor_api_session.py` | steps/inputs/artifacts with supported refs | Command result and YAML golden | P0 | High drift risk. |
| Structured params | `tests/test_editor_api_step_specs.py`, `apps/config-editor/tests/stepParams.logic.test.ts`, `tests/test_editor_core.py` | grant permissions, artifact lists, policy params | DTO and command parity | P1 | Preserve extra keys where current behavior does. |
| Advanced internals | `tests/test_editor_api_session.py`, `apps/config-editor/tests/advancedStepInternals.logic.test.ts` | constraints, `skip_if`, `verify` JSON | Command parity | P1 | Nested refs remain literals. |
| Planner | `tests/test_planner_core.py`, `tests/test_cli.py` | draft plan and execution plan goldens | Golden YAML/DTO/struct output | P2 | Port after editor backend parity. |
| Executor | `tests/test_executor_core.py`, `tests/test_cli.py` | dry-run and mocked ADB cases | Rust integration/unit tests | P3 | Port late due device/process risk. |
| CLI | `tests/test_cli.py` | command stdout/stderr and exit codes | CLI process tests | P3 | Keep Python CLI until core parity. |

Golden fixture strategy:

- Authored YAML fixtures.
- Expected canonical emitted YAML.
- Expected diagnostics DTOs.
- Expected refIndex DTOs.
- Expected StepSpecDto output.
- Expected command result/document DTO sequences.
- Expected planner draft and execution-plan YAML.
- Expected executor dry-run summaries and result records.

## Staged Roadmap

| Phase | Goal | Risk | Prerequisites |
| --- | --- | --- | --- |
| 6C | Rust backend skeleton: `hello`, protocol version, capabilities, envelopes/errors, JSONL loop, one-shot hello. No document editing. | Low | Confirm future crate location; establish hello parity fixture first. |
| 6D | DTO/schema/StepSpec parity. Static built-in step registry metadata and `listStepSpecs`. | Medium | Golden `StepSpecDto`; model param shapes and ref filters. |
| 6E | YAML load/emit plus validation skeleton. Load recipes, emit canonical YAML, basic diagnostics. | High | YAML crate choice; order-preserving model; golden YAML/diagnostics. |
| 6F | Editor document sessions. Open/get/save, dirty/canUndo/canRedo, undo/redo snapshot strategy. | High | RecipeDocument model and canonical YAML parity. |
| 6G | Editor command parity. Non-step commands, step lifecycle, dependencies, params, advanced internals, safe delete. | High | Command codec tests; mutation goldens; safe-delete fixtures. |
| 6H | Ref index parity. Complete `RefIndexDto` and candidate behavior. | Medium | Registry output metadata and representative recipe fixtures. |
| 6I | Planner parity. Draft sessions, dependency expansion, binding behavior, execution-plan emission. | High | Full catalog fixtures and golden planning results. |
| 6J | Executor parity. Dry-run, mocked ADB, step handlers, device/file/network behavior. | Very high | Planner parity and expanded executor fixture coverage. |
| 6K | Backend selection/cutover and packaging. Decide sidecar binary selection, packaging, and removal timeline. | High | Agreed parity suite passing; explicit product decision. |

## Defer / Do Not Port Yet

Do not port these in early editor backend phases:

- Executor: high device, shell, network, filesystem, and platform risk.
- Production packaging: no Python bundling or Rust sidecar distribution in Phase
  6B through early parity phases.
- CLI replacement: keep Python CLI until Rust planner/executor parity exists.
- Dynamic external plugin behavior: not implemented in Python and not needed for
  parity.
- Python removal: Python remains reference implementation.
- PySide6 removal: legacy PySide6 source, tests, dependency metadata, and GUI
  entrypoints are removed.
- Backend selection UI: do not add until Rust sidecar has meaningful parity.
- Save As UI: sidecar supports it, Tauri UI intentionally does not expose it.
- Executor/apply-device UI: outside this migration phase.
- Direct YAML editing: outside the authored-model-first editor contract.
- Any subsystem without golden fixtures: add fixtures before trusting a Rust
  replacement.

## Risk Register

| Risk | Likelihood | Impact | Mitigation | Parity tests |
| --- | --- | --- | --- | --- |
| YAML formatting/order drift | High | High | Deliberate ordered payload builder; golden emitted YAML tests. | YAML load/emit matrix. |
| Unknown field preservation mismatch | Medium | High | Document Python behavior, preserve known unsupported step content, mark top-level unknowns as open question. | Unsupported step-content fixtures. |
| Python dynamic behavior vs Rust static types | Medium | Medium | Use flexible values for params/condition params; static registry first. | Command codec and structured params tests. |
| Validation diagnostics drift | High | High | Golden diagnostic DTOs, include ordering and field paths. | Validation matrix. |
| Planner execution order drift | Medium | High | Topological-order goldens and draft-plan fixtures. | Planner matrix. |
| Executor device side effects | High | High | Defer; start with dry-run/mocked ADB before real device. | Executor dry-run and mocked ADB tests. |
| Undo/redo semantic differences | Medium | High | Start with full snapshot strategy. | Undo/redo matrix. |
| Ref index mismatch | Medium | Medium | Port after registry metadata; compare full DTO order. | Ref index matrix. |
| Step spec metadata drift | Medium | High | Snapshot full `StepSpecDto`; make registry source explicit. | Step specs matrix. |
| Sidecar protocol drift | Medium | High | First fixture is hello/protocol/capabilities; compare envelopes. | Hello/protocol and envelope tests. |
| Test fixture gaps | High | High | Build dual-backend harness before replacement. | Harness coverage report. |
| Packaging/sidecar distribution complexity | Medium | Medium | Defer to 6K; keep development sidecar external. | Packaging acceptance tests later. |
| Tauri compatibility gate blocks read-only dogfood | High | Medium | Dogfood read-only in harness first, or add later explicit read-only/backend selection mode only after approval. | Current-Tauri dogfood checklist. |

## Open Questions

- Does Rust need to preserve YAML comments if Python does not?
- Should Rust intentionally preserve unknown top-level authored fields that
  Python canonical emission does not currently preserve?
- Should the Rust backend remain a sidecar permanently or move in-process after
  parity?
- Should the step registry stay static in Rust until external plugins are
  designed?
- Which YAML crate is acceptable after golden output comparison?
- What exact capability set is required for the first Rust dogfood outside the
  current Tauri compatibility gate?
- When, if ever, should Python backend removal be considered?
- How much executor behavior must be ported before production packaging is
  worth planning?
- Should parity harness goldens compare emitted YAML byte-for-byte or compare
  parsed semantic payloads plus selected formatting invariants?

## Parity Before Replacement Rule

At Phase 6B planning time, Python remained the reference backend until Rust
passed agreed dual-backend parity tests. Rust was not to replace Python by
default during early port phases, and the then-current Tauri backend selection
and sidecar protocol were to remain unchanged until a later cutover phase
explicitly approved replacement.

## Phase 6B Validation Notes

Phase 6B should not run the full test suite unless docs tooling requires it.
The required validation for this docs-only phase is:

- Confirm this file exists at `docs/rust-backend-port-plan.md`.
- Confirm no source, tests, package files, app behavior, protocol behavior, or
  backend selection changed.
- Confirm all current repo paths referenced as existing paths resolve.
- Confirm all speculative Rust crate paths are clearly marked as proposed
  future paths.
