# EmuChef

CLI-first Android emulation handheld provisioner.

## Context

Repository context and current behavior notes live in `CONTEXT.md`.

## Editor

The desktop editor targets authored recipe files only.

Milestone 2 supports:

- Overview editing
- Inputs editing
- Artifacts editing
- Artifact Groups editing
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
- step editing and ref rewrite are not implemented yet

## Templates

Example authored YAML templates live under `templates/authored/`.
They are examples for authors only and are not loaded by the CLI.

To create real authored inputs, copy a template into the matching `authored/`
subdirectory:

- `templates/authored/app_definition.template.yaml` -> `authored/apps/`
- `templates/authored/recipe.template.yaml` -> `authored/recipes/`
- `templates/authored/device_profile.template.yaml` -> `authored/device_profiles/`
- `templates/authored/device_plan.template.yaml` -> `authored/device_plans/`
