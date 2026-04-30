# EmuChef

CLI-first Android emulation handheld provisioner.

## Context

Repository context and current behavior notes live in `CONTEXT.md`.

## Step Architecture

Supported steps are first-party built-in plugins registered in the in-repo step
registry. The registry is the source of truth for step specs, planner hooks,
executor handler lookup, primary outputs, and editor-safe metadata.

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
pip install -e .
emuchef-editor /path/to/EmuChef
```

Current editor scope notes:

- it edits the shared typed authored recipe model rather than raw YAML text
- `kind` and `schema_version` are read-only
- input, artifact, and group ids are chosen at creation time and then stay read-only
- step ids and step types are chosen at creation time and then stay read-only
- permission editing lives on `grant_permissions` step params as `runtime`, `appops`, and `policy`
- top-level recipe `permissions:` is invalid and is not migrated or ignored by the loader
- step refs stay in authored-ref space and save explicitly as `{ ref: ... }`
- the Steps page uses structured pickers for dependencies, refs, constraints, `skip_if`, and `verify`
- unsupported authored step content that the current M4 UI does not edit is preserved rather than dropped
- deleting a step does not rewrite downstream dependencies or refs; diagnostics surface the resulting breakage
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
