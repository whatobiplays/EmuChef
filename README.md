# EmuChef

CLI-first Android emulation handheld provisioner.

## Templates

Example authored YAML templates live under `templates/authored/`.
They are examples for authors only and are not loaded by the CLI.

To create real authored inputs, copy a template into the matching `authored/`
subdirectory:

- `templates/authored/app_definition.template.yaml` -> `authored/apps/`
- `templates/authored/recipe.template.yaml` -> `authored/recipes/`
- `templates/authored/device_profile.template.yaml` -> `authored/device_profiles/`
- `templates/authored/device_plan.template.yaml` -> `authored/device_plans/`
