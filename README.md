# EmuChef

CLI-first Android emulation handheld provisioner.

## Context

Repository context and current behavior notes live in `CONTEXT.md`.

## Editor

The Milestone 1 desktop editor targets authored recipe files only.

Install the project in development mode, then launch the editor against either:

- a repo root that contains `authored/`
- the `authored/` root itself

Example:

```bash
pip install -e .
emuchef-editor /path/to/EmuChef
```

## Templates

Example authored YAML templates live under `templates/authored/`.
They are examples for authors only and are not loaded by the CLI.

To create real authored inputs, copy a template into the matching `authored/`
subdirectory:

- `templates/authored/app_definition.template.yaml` -> `authored/apps/`
- `templates/authored/recipe.template.yaml` -> `authored/recipes/`
- `templates/authored/device_profile.template.yaml` -> `authored/device_profiles/`
- `templates/authored/device_plan.template.yaml` -> `authored/device_plans/`
