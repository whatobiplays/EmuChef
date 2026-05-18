# EmuChef

CLI-first Android emulation handheld provisioner.

## Context

Repository context and current behavior notes live in `CONTEXT.md`.
The current Rust/Python ownership decision is recorded in
[ADR 0001](docs/adr/0001-rust-tauri-editor-runtime-ownership.md).

Authored YAML under `authored/` is the source for current planning behavior.
Generated execution-plan YAML is emitted on demand and is not maintained as
root-level `*.plan.yaml` examples.

## Step Architecture

Supported steps are first-party built-in plugins registered in the in-repo step
registry. The registry is the source of truth for step specs, planner hooks,
direct executor handler callables, primary outputs, and editor-safe metadata.
Any remaining `StepSpec.executor_handler` values are transitional metadata only;
runtime dispatch uses the registry plugin handler callable.

Step type ids are plain registry-owned strings. Authored recipes and execution
plans still use the same visible YAML values, such as `copy_files` and
`grant_permissions`, and `schema_version: 1` remains current. External plugin
loading is deferred follow-up work.

## Editor

The Tauri config editor in `apps/config-editor` is the primary development UI
for authored recipe files. It edits the shared typed authored recipe model
through the Rust sidecar and does not provide direct YAML editing.

The legacy PySide6 editor remains available only as temporary
legacy/reference/developer tooling for comparison and debugging. It is not the
active editor runtime or a Tauri fallback.

The UI-free Python editor API is available without the `pyside-editor` extra for
legacy/reference and golden-generation workflows. It is not used by the Tauri
editor runtime. The one-shot server accepts one JSON request per process
invocation:

```bash
python -m emuchef_editor.api.server '{"type":"listStepSpecs"}'
python -m emuchef_editor.api.server '{"type":"hello"}'
echo '{"type":"openRecipe","payload":{"path":"authored/recipes/app.retroarch.provision.yaml","authoredRoot":"authored"}}' | python -m emuchef_editor.api.server
```

The same Python reference entrypoint also supports a persistent JSON Lines
sidecar for development clients that need reusable document sessions:

```bash
printf '%s\n' '{"id":"req-1","type":"listStepSpecs","payload":{}}' | python -m emuchef_editor.api.server --sidecar
printf '%s\n' '{"id":"req-1","type":"hello","payload":{}}' | python -m emuchef_editor.api.server --sidecar
```

Sidecar stdin accepts one JSON request per line and stdout returns one JSON
response per line. Stdout is machine-readable JSONL only; diagnostics belong on
stderr. Every valid sidecar request includes an opaque string `id`, and every
response echoes that id. Malformed JSON lines return `id: null`. The backend
protocol includes a `hello` request that returns `protocolVersion: 1` and a
backend-agnostic `capabilities` string list. The protocol does not expose
`implementation` or `implementationVersion` fields, and there is no protocol
negotiation yet. Future Rust backends should implement the same protocol.

### Tauri config editor

The Tauri editor is a Tauri v2 development app that uses the experimental Rust
JSONL sidecar for session-backed document operations. Python remains in the repo
only for legacy/reference/developer/golden workflows such as the PySide editor,
Python CLI reference behavior, fixture/golden generation, and later
confirmed-cutover work.

Install frontend dependencies and run the Tauri dev shell with npm:

```bash
cd apps/config-editor
npm install
npm run tauri dev
```

The Tauri dev/build hooks prepare the Rust sidecar automatically. App-local
details live in `apps/config-editor/README.md`.

```bash
npm run tauri build
```

During development, the Rust bridge discovers the repo root and checks the
crate-local and repo-root Cargo `target/debug` directories for
`emuchef-rust-backend`. Packaged builds bundle the Rust sidecar through Tauri v2
`externalBin`.

Current editor scope notes:

- it edits the shared typed authored recipe model rather than raw YAML text
- the Tauri editor supports sidecar-backed editing for Overview, Inputs, Artifacts, Artifact Groups, basic step lifecycle operations, step dependencies, schema-backed rich step params, typed ref picking, and advanced JSON-backed step internals for constraints, `skip_if`, and `verify`
- primary document actions are exposed through native File, Edit, and Utilities menus
- menu items are context-aware and disabled when a document action is not valid, a command is in flight, or the sidecar session is invalid
- the Rust sidecar client starts the backend on the first real sidecar request, sends `hello`, requires protocol version 1 and the editor's required capabilities, and then continues the original request without a frontend retry
- `sidecar_status` is local Rust process/session state; it does not start the sidecar or send `hello`, and it reports cached compatibility metadata after a sidecar process has been started
- incompatible backends stop normal document requests, mark the session invalid, and leave any stale document visible only as read-only reference
- the Tauri Save command writes the current sidecar document to disk and should be tested only on safe or temporary recipe copies during development
- native confirmation prompts guard opening another recipe with unsaved changes and closing the window/app with unsaved changes or an operation in flight where Tauri close interception is available
- if the Rust sidecar exits or transport fails, the stale document remains visible for reference, document-specific actions are disabled, and the explicit Restart Sidecar action can recover a compatible running sidecar before the recipe is explicitly reopened
- create-from-template sidecar capabilities are not exposed in the Tauri UI
- capability names are backend-agnostic protocol strings; optional capabilities are display/status metadata only in the current UI
- production signing, notarization, updater support, and cross-platform release automation are not implemented
- there is no backend selector, backend toggle, config option, environment variable, UI switch, protocol negotiation path, or Python fallback for the Tauri editor runtime
- executor/apply-device UI is not implemented
- `id`, `kind`, and `schema_version` are read-only in the Overview screen
- input, artifact, and group ids are changed through explicit Rename actions
- artifact groups can be duplicated; the duplicate starts with the same ordered artifact members as the source group
- step ids and step types are chosen only when adding a step and then stay read-only
- basic step lifecycle editing uses Rust sidecar commands for add, delete, duplicate, reorder, display-name edits, and `user_toggleable` edits
- step dependency editing uses the Rust sidecar `UpdateStepDependencies` command; adding appends a dependency id as authored storage/display order only, and the planner remains authoritative for final execution ordering
- missing or unknown authored dependency ids remain visible in the step detail panel and can be removed from copied or temporary recipes during repair
- deleting a step uses the backend safe-delete behavior shared with the PySide editor and removes supported downstream dependencies, conflicts, and refs
- step params editing uses the Rust sidecar `UpdateStepParams` command with full params replacement for the selected step; the frontend submits authored JSON values and replaces local document state with the returned `RecipeDocumentDto`
- step params with known Python schema shapes use rich controls where safe: ordered artifact and artifact-group id list params use add/remove/up/down list controls, runtime and app-op permission params use row editors, and policy params use select/checkbox controls
- raw JSON remains the editor for free-form, unsupported, incompatible, or schema-less params such as metadata; schema-backed structured editors preserve unknown or extra object keys where the authored value can be copied without data loss
- refs use the authored `{ ref: "..." }` shape in DTOs and command payloads; the backend codec converts only top-level exact ref-shaped param values into internal domain refs
- the ref picker uses the current document `refIndex`, prefers candidate metadata, falls back to raw `allRefs`, and keeps missing or incompatible current refs visible for repair
- `StepSpecDto` improves param ordering, enum rendering, ref filtering, and known param shape rendering, but it is UI metadata only and is not mutation authority
- backend validation remains authoritative for required params, ref validity, and step contract diagnostics
- editable Tauri text controls disable browser writing aids and normalize smart single and double quotes to ASCII quotes before storing local drafts or sending sidecar commands
- selecting a ref does not automatically add or rewrite step dependencies in the Tauri editor
- constraints, `skip_if`, and `verify` use plain JSON editors in an Advanced step section; each edit parses JSON locally, requires explicit Apply, and submits `UpdateStepConstraints`, `UpdateStepSkipIf`, or `UpdateStepVerify` through the Rust sidecar
- the constraints JSON editor displays authored/YAML-facing `conflicts_with`; the command payload still uses the API field `conflictsWith`
- advanced step JSON editors do not provide specialized constraints, condition, or verification builders, and they do not provide a ref picker inside advanced JSON values
- backend command application and validation remain authoritative for advanced step internals; local frontend checks are limited to JSON parsing, representable authored keys, and the top-level shape required by the command codec
- advanced JSON command success with `changed: false` is treated as a no-op, not an applied edit
- Inputs, Artifacts, Artifact Groups, and Steps use independently scrolling list/detail panes with resizable list columns
- top-level recipe `permissions:` is invalid and is not migrated or ignored by the loader
- step refs stay in authored-ref space and save explicitly as `{ ref: ... }`
- the YAML preview remains read-only and is refreshed through the sidecar session
- unsupported authored step content that the current Tauri UI does not edit is preserved rather than dropped
- ref rewrite after id changes is not implemented

### Legacy PySide6 editor

The PySide6 editor is installed with the optional `pyside-editor` extra and
continues to use the existing `emuchef-editor` script entrypoint:

```bash
pip install -e ".[pyside-editor]"
emuchef-editor /path/to/EmuChef
```

Use the PySide6 editor only as legacy/reference/developer tooling for comparison
and debugging. It remains optional and is not required for the UI-free editor API
or the Tauri development editor.

## Templates

Example authored YAML templates live under `templates/authored/`.
They are examples for authors only and are not loaded by the CLI as real authored inputs.

The legacy PySide6 editor shows recipe templates separately from authored
recipes and uses them as creation sources for `New Recipe...`. Template preview
in that editor is read-only and informational.

To create real authored inputs, copy a template into the matching `authored/`
subdirectory:

- `templates/authored/app_definition.template.yaml` -> `authored/apps/`
- `templates/authored/recipe.blank.template.yaml` -> `authored/recipes/`
- `templates/authored/recipe.template.yaml` -> `authored/recipes/`
- `templates/authored/device_profile.template.yaml` -> `authored/device_profiles/`
- `templates/authored/device_plan.template.yaml` -> `authored/device_plans/`
