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

The code is intentionally split into:

- `src/emuchef/io`: authored loading, validation, YAML I/O
- `src/emuchef/planner`: normalization, draft/session logic, execution-plan emission
- `src/emuchef/executor`: runtime ref resolution, ADB, shared step runtime, and shared execution helpers
- `src/emuchef/steps`: first-party built-in step plugins, step specs, planner hooks,
  step-local executor handlers, and editor-safe step metadata
- `src/emuchef/domain`: typed models and enums
- `src/emuchef_editor/core`: UI-agnostic recipe document, canonical YAML, ref indexing, and validation adapters for the editor
- `src/emuchef_editor/api`: UI-free JSON API adapters over the editor core
- `src/emuchef_editor/app`: legacy/fallback PySide6 desktop editor for authored recipe files
- `apps/config-editor`: primary Tauri development/editor UI for authored recipe files

The base Python package contains non-UI runtime dependencies only. The PySide6
desktop editor is installed with the `pyside-editor` optional dependency extra
and remains available for comparison and debugging.

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

## Current Editor API

`emuchef_editor.api` is the UI-free JSON API surface for editor clients. It
wraps `src/emuchef_editor/core` and keeps Python authoritative for authored
recipe loading, command application, validation, ref indexing, canonical YAML
emission, saving, and step registry metadata.

Every API response uses one of these envelopes:

- `{"ok": true, "result": {...}}`
- `{"ok": false, "error": {"code": "...", "message": "...", "details": {...}}}`

Failure responses include diagnostic debug details only when a request sets
`debug: true`. Frontends treat debug details as diagnostics, not behavior
contracts.

`RecipeDocumentDto` contains document state: document id, path, authored root,
dirty state, undo/redo availability, recipe DTO, current canonical YAML,
diagnostics, and ref index. It does not contain step specs.

`RecipeDto` exposes current authored recipe sections for overview, dependencies,
provided features, inputs, artifacts, artifact groups, and steps. Top-level
permissions are invalid authored data and are absent from `RecipeDto`.
Permission authoring appears as normal step data on `grant_permissions` steps.

Step specs are returned only by the `listStepSpecs` API request. Step spec data
comes from the built-in step registry and includes editor-safe labels, supported
status, outputs, param ordering, defaults, and typed ref filter hints where the
registry exposes them.

The API server supports one-shot stateless requests through:

- `listStepSpecs`
- `openRecipe`
- `validateRecipePath`
- `emitRecipeYamlFromPath`

`openRecipe` may return a document id from the one-shot server, but only the
sidecar-backed `DocumentSessionManager` owns reusable document sessions.

The API server also supports a persistent JSON Lines sidecar mode through:

```bash
python -m emuchef_editor.api.server --sidecar
```

The sidecar reads one UTF-8 JSON request per stdin line and writes one UTF-8
JSON response per stdout line. Stdout is machine-readable JSONL only; diagnostics
and human-readable logs belong on stderr. Every valid sidecar request includes
an opaque string id, every response echoes that id exactly, and malformed JSON
lines return `id: null`. Sidecar request-level failures use the same `ok:false`
API envelope as one-shot requests and do not terminate the process. The sidecar
exits cleanly when stdin reaches EOF.

The sidecar reuses `DocumentSessionManager` for live document sessions. It
supports session-backed requests for listing step specs, opening and creating
documents, getting and closing documents, applying recipe commands, undo, redo,
saving, Save As, validation, canonical YAML emission, and ref-index retrieval.
The sidecar protocol includes a backend-agnostic `hello` request. `hello`
returns integer `protocolVersion: 1` and a string `capabilities` list. The
protocol does not expose `implementation` or `implementationVersion` fields,
and there is no protocol negotiation. Capability names describe editor protocol
operations rather than backend implementation details. The current Python
backend reports the capabilities it supports, including optional protocol
operations such as create-from-template, document close, and Save As.

## Current Tauri Config Editor

`apps/config-editor` is the primary Tauri development/editor UI for authored
recipe files. It uses Tauri v2, React, TypeScript, Vite, Tailwind, npm, and the
official Tauri dialog plugin. The app is a development editor shell, not a
production-packaged application.

The editor opens authored recipe YAML files through a native file picker, calls
the Python sidecar through Rust Tauri commands, and displays recipe data,
diagnostics, canonical YAML, sidecar status, and available step specs. It keeps a
reusable `documentId` for the currently open sidecar document. After a document
is opened through the sidecar, document-specific actions use document-id-based
sidecar requests so unsaved in-memory edits remain visible to validation, YAML
emission, undo, redo, and save.

The Tauri editor runs in development mode with a local Python interpreter. The
Rust bridge uses `EMUCHEF_PYTHON` when set and otherwise uses `python`. During
development it discovers the repo root and prepends `src/` to `PYTHONPATH`; if
repo discovery is unavailable, the selected Python must already be able to
import the local `emuchef_editor` package. The editor does not bundle Python,
create installers, sign/notarize builds, configure updates, or solve production
sidecar distribution.

The Tauri editor supports sidecar-backed recipe editing. The Overview screen
edits recipe name and description. Recipe id, schema version, and kind are
read-only in the Overview screen. Inputs, artifacts, and artifact groups support
CRUD-style editing where the Python command codec exposes the corresponding
document command. Add, rename, and duplicate actions collect required text with
app-owned dialogs, destructive actions require confirmation, and artifact
creation collects both artifact id and URL before submitting a command. Artifact
group duplication creates a new group with the same ordered artifact members as
the source group and does not rewrite step refs or selections.

The Tauri Steps screen supports basic step lifecycle editing, step dependency
editing, existing step param editing, and JSON-backed advanced step internals
editing through the Python sidecar. Step add, delete, duplicate, reorder,
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
execution ordering to Python validation and planning. Missing or unknown
authored dependency ids remain visible as raw ids and can be removed. Delete Step
uses the backend safe-delete behavior shared with the PySide editor, so
supported downstream step dependencies, `conflicts_with` entries, and step refs
are removed by Python rather than by TypeScript cleanup logic.

Step param updates use `UpdateStepParams` with the complete next params object
for the selected step. The frontend edits authored step params, keeps literal
values as JSON primitives, objects, or lists, and sends refs in the authored
`{ ref: "..." }` shape. A literal JSON `null` param value is distinct from
clearing a param; clearing removes the param key from the submitted params
object. The Python backend remains authoritative for command application,
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
unless a Python schema is added for it later.

The ref picker uses the current document `refIndex`, prefers `candidates`,
falls back to `allRefs`, and keeps current missing or incompatible refs visible
for repair. Selecting a ref does not automatically add or rewrite step
dependencies.

Advanced step internals use a collapsed Advanced section with plain JSON
textarea editors for constraints, `skip_if`, and `verify`. Each editor keeps a
local draft until the user selects Apply, parses JSON before submitting, and
then sends `UpdateStepConstraints`, `UpdateStepSkipIf`, or `UpdateStepVerify`
through `sidecar_apply_recipe_command`. The constraints editor displays the
authored/YAML-facing `conflicts_with` key, and the frontend converts that key to
the API command field `conflictsWith` only when submitting
`UpdateStepConstraints`. Revert restores the draft from the current returned
document value. JSON `null` is a literal JSON value and is not a clear action;
if the command codec requires a different top-level shape, the frontend reports
a local shape error and does not submit. Advanced JSON values that contain
objects shaped like `{ "ref": "..." }` are ordinary JSON values in this editor.
The Tauri editor has no specialized constraints builder, `skip_if` condition
builder, `verify` builder, or ref picker inside advanced JSON values. Python
remains authoritative for advanced internals command application, canonical
YAML, semantic validation diagnostics, dirty state, undo state, and redo state.
Advanced internals command success with `changed: false` is treated as a no-op
rather than an applied edit.

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
YAML, or DTO data beyond the returned document state from Python.

The Tauri editor exposes primary app actions through native menus. File contains
Open Recipe and Save. Edit contains Undo and Redo. Utilities contains Validate
and Refresh YAML. Menu items are context-aware and disabled when a document
action is not valid, the document session is invalid, or a conflicting command
is in flight. The app follows Tauri v2 desktop menu behavior: macOS uses the
native app menu convention, while Windows and Linux use native window menu bars.
Debug-only controls are not exposed in the normal menu/UI path.

Native confirmation prompts guard opening another recipe with unsaved changes
and closing the Tauri window with unsaved changes or an operation in flight when
Tauri close-request interception is available. Dirty open confirmation happens
before the native file picker, file-picker cancellation keeps the current
document/session unchanged, and failed opens preserve the current document when
the old session remains valid. Dirty close handling uses frontend
`RecipeDocumentDto.dirty` state and does not save implicitly. Close handling
does not cancel in-flight operations.

The Tauri editor does not expose dependency graph visualization, dependency
reorder controls, drag-and-drop, executor/apply-device UI, Save As UI,
create-from-template UI, Python bundling, installer packaging, or Rust ports of
Python editor/planner/executor behavior. YAML preview is read-only.

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

The Rust bridge keeps the Phase 2 one-shot Python API bridge available and also
owns one persistent sidecar process for sidecar-backed commands. The sidecar
client starts Python on the first sidecar request, sends `hello`, requires
protocol version 1 and the editor's required capabilities, and then continues
the original request without a frontend retry. A `Running` Rust sidecar state
means the process is started and handshake-compatible. The client serializes
requests as one send-line/read-line operation at a time and treats non-handshake
API `ok:false` envelopes as successful Rust transport results. `EMUCHEF_PYTHON`
selects the Python command when set; otherwise the bridge uses `python`. During
development, the bridge discovers the repo root and prepends `src/` to
`PYTHONPATH` so the selected Python command can import the local package.

`sidecar_status` reports local Rust process and compatibility state only. It
does not start the sidecar and does not perform a fresh `hello` call. Before the
first sidecar request, compatibility is unchecked. After a sidecar process has
started, status reports cached compatibility metadata such as compatibility,
protocol version, capabilities, and the last compatibility or transport error.
If the sidecar exits unexpectedly, an unrecoverable transport failure occurs, or
the backend is incompatible, the frontend marks the document session invalid,
leaves the stale document visible for reference, disables or short-circuits
document-specific actions, and tells the user to restart the Tauri app and
reopen the recipe. The sidecar does not automatically restart, and previous
document ids are invalid after process loss.

Production packaging, Python bundling, and a Rust backend replacement are future
work. A future Rust backend must implement the same backend-agnostic editor
protocol rather than changing the Tauri editor to backend-specific request
shapes.

`crates/emuchef-rust-backend` is an experimental standalone Rust backend
skeleton for migration work. It currently implements `hello`, response
envelopes, one-shot requests, JSON Lines sidecar requests, and `listStepSpecs`.
Its `hello` response reports only `["listStepSpecs"]`, which does not make it
compatible with the Tauri editor because required document capabilities are
missing. Rust `listStepSpecs` uses a temporary Python-generated static fixture
for StepSpec DTO parity; Python remains the reference implementation until a
Rust backend replacement is explicitly approved. The fixture-backed source is
scaffolding only and should be replaced with Rust-native schema builders before
planner or executor behavior is ported.

The frontend passes `authoredRoot: null` for Phase 3A. Explicit authored-root and
workspace selection are deferred editor migration work.

## Current PySide6 Legacy Editor

The PySide6 editor remains available through the `pyside-editor` optional
dependency extra and the existing `emuchef-editor` script entrypoint. It is a
legacy/fallback editor path for comparison and debugging, not the primary editor
path. The PySide6 code stays in the repo and continues to use the shared Python
editor core, authored recipe model, canonical YAML writer, and validation
adapters.

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
- new recipes are created from recipe templates under `templates/authored/`, including a blank recipe template
- template preview is read-only and informational
- the workspace lists authored recipes separately from recipe templates
- the workspace list auto-refreshes when authored recipe files or recipe template files are added, removed, or renamed on disk while the workspace is open
- if an open recipe file disappears from the workspace because of an external remove or rename, the in-memory document stays open and the workspace selection clears until that exact path reappears
- unsaved-changes prompts gate opening another recipe, starting a new recipe, and closing the window
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

The recipe editor treats recipe templates as creation sources, not as normal
editable authored recipe documents.
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
