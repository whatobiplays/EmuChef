# EmuChef

CLI-first Android emulation handheld provisioner.

## Context

Repository context and current behavior notes live in `CONTEXT.md`.

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

The desktop editor targets authored recipe files only.

The current editor supports:

- Overview editing
- Inputs editing
- Artifacts editing
- Artifact Groups editing
- Steps editing for the current supported step types:
  - `resolve_artifacts`
  - `extract_artifacts`
  - `extract_archive`
  - `copy_files`
  - `install_apk`
  - `grant_permissions`
  - `launch_app`
  - `wait`
  - `force_stop_app`
- template-driven New Recipe creation
- blank-template New Recipe creation
- Save As
- template browsing as creation sources
- unsaved-changes prompts for open/new/close flows
- immediate in-memory document updates
- explicit save-to-disk
- live diagnostics and canonical YAML preview refresh
- command-level undo and redo that survives save

Install the project in development mode, then launch the editor against either:

- a repo root that contains `authored/`
- the `authored/` root itself

Example:

```bash
pip install -e ".[pyside-editor]"
emuchef-editor /path/to/EmuChef
```

The UI-free editor API is available without the `pyside-editor` extra. The
one-shot server accepts one JSON request per process invocation:

```bash
python -m emuchef_editor.api.server '{"type":"listStepSpecs"}'
echo '{"type":"openRecipe","payload":{"path":"authored/recipes/app.retroarch.provision.yaml","authoredRoot":"authored"}}' | python -m emuchef_editor.api.server
```

The same entrypoint also supports a persistent JSON Lines sidecar for
development clients that need reusable document sessions:

```bash
printf '%s\n' '{"id":"req-1","type":"listStepSpecs","payload":{}}' | python -m emuchef_editor.api.server --sidecar
```

Sidecar stdin accepts one JSON request per line and stdout returns one JSON
response per line. Stdout is machine-readable JSONL only; diagnostics belong on
stderr. Every valid sidecar request includes an opaque string `id`, and every
response echoes that id. Malformed JSON lines return `id: null`. The sidecar has
no `protocolVersion` or ping request yet; revisit protocol versioning before
replacing the Python backend or externalizing the protocol.

### Tauri config editor

The config editor lives in `apps/config-editor`. It is a Tauri v2 app that uses
the persistent Python JSONL sidecar for session-backed document operations. The
one-shot Python API remains available for compatibility and regression safety.
Python remains authoritative for authored recipe mutations.

Install frontend dependencies and run the Tauri dev shell with npm:

```bash
cd apps/config-editor
npm install
npm run tauri dev
```

The app uses the local Python package through `python -m
emuchef_editor.api.server --sidecar`. Set `EMUCHEF_PYTHON` when the default
`python` command is not the interpreter that can import EmuChef:

```bash
EMUCHEF_PYTHON=../../.venv/bin/python npm run tauri dev
```

During development, the Rust bridge discovers the repo root and prepends
`<repo>/src` to `PYTHONPATH`. If repo discovery is unavailable, the selected
Python environment must already be able to import the local `emuchef_editor`
package.

Current editor scope notes:

- it edits the shared typed authored recipe model rather than raw YAML text
- the Tauri editor supports sidecar-backed editing for Overview, Inputs, Artifacts, Artifact Groups, basic step lifecycle operations, and step dependencies
- primary document actions are exposed through native File, Edit, and Utilities menus
- temporary development-only actions are exposed through the native Debug menu
- menu items are context-aware and disabled when a document action is not valid
- the Tauri Save command writes the current sidecar document to disk and should be tested only on safe or temporary recipe copies during development
- window/app close unsaved-change handling remains later-phase work
- Save As and create-from-template sidecar capabilities are not exposed in the Tauri UI
- `id`, `kind`, and `schema_version` are read-only in the Overview screen
- input, artifact, and group ids are changed through explicit Rename actions
- artifact groups can be duplicated; the duplicate starts with the same ordered artifact members as the source group
- step ids and step types are chosen only when adding a step and then stay read-only
- basic step lifecycle editing uses Python sidecar commands for add, delete, duplicate, reorder, display-name edits, and `user_toggleable` edits
- step dependency editing uses the Python sidecar `UpdateStepDependencies` command; adding appends a dependency id as authored storage/display order only, and the planner remains authoritative for final execution ordering
- missing or unknown authored dependency ids remain visible in the step detail panel and can be removed from copied or temporary recipes during repair
- deleting a step uses the backend safe-delete behavior shared with the PySide editor and removes supported downstream dependencies, conflicts, and refs
- params, constraints, `skip_if`, and `verify` are read-only in the Tauri editor
- full step params and ref editing remain later-phase work
- Inputs, Artifacts, Artifact Groups, and Steps use independently scrolling list/detail panes with resizable list columns
- top-level recipe `permissions:` is invalid and is not migrated or ignored by the loader
- step refs stay in authored-ref space and save explicitly as `{ ref: ... }`
- the YAML preview remains read-only and is refreshed through the sidecar session
- unsupported authored step content that the current Tauri UI does not edit is preserved rather than dropped
- ref rewrite after id changes is not implemented

## Templates

Example authored YAML templates live under `templates/authored/`.
They are examples for authors only and are not loaded by the CLI as real authored inputs.

The editor shows recipe templates separately from authored recipes and uses them as creation sources for `New Recipe...`.
Template preview in the editor is read-only and informational.

To create real authored inputs, copy a template into the matching `authored/`
subdirectory:

- `templates/authored/app_definition.template.yaml` -> `authored/apps/`
- `templates/authored/recipe.blank.template.yaml` -> `authored/recipes/`
- `templates/authored/recipe.template.yaml` -> `authored/recipes/`
- `templates/authored/device_profile.template.yaml` -> `authored/device_profiles/`
- `templates/authored/device_plan.template.yaml` -> `authored/device_plans/`
