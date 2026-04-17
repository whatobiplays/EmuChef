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
- `src/emuchef/executor`: runtime ref resolution, artifact handling, ADB, step execution
- `src/emuchef/domain`: typed models and enums
- `src/emuchef_editor/core`: UI-agnostic recipe document, canonical YAML, ref indexing, and validation adapters for the editor
- `src/emuchef_editor/app`: PySide6 desktop editor for authored recipe files

## Current Authored Model

Recipes now use:

- `inputs:` as a map keyed by input id
- `artifacts:` as a map keyed by artifact id
- `artifact_groups:` as a map keyed by group id
- top-level declarative `permissions:`
- ordered `steps:`

Author refs stay recipe-local in YAML:

- `inputs.<id>`
- `artifacts.<id>.<field>`
- `steps.<id>`
- `steps.<id>.outputs.<field>`

Planner-internal execution-plan refs are normalized and namespaced, but that
does not leak into authored YAML.

Literal params are authored directly. Only refs use `{ ref: ... }`.

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

It does not yet edit steps or rewrite refs after id changes.

Editor interaction rules:

- edits apply immediately to the in-memory recipe document
- save is explicit and writes canonical YAML to disk
- diagnostics and YAML preview refresh after each committed edit
- undo and redo operate at command granularity and persist across saves for the open document
- dirty state is a semantic comparison against the last saved canonical YAML baseline

Field-scope rules currently enforced by the editor:

- `kind` is read-only
- `schema_version` is read-only and reflects latest-schema-only support
- artifact kind support is currently limited to `remote_file`
- input, artifact, and artifact-group ids are chosen at creation time and then remain read-only
- deleting inputs, artifacts, or groups does not rewrite step refs
- deleting an artifact removes it from artifact-group memberships, but does not rewrite step refs

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

Executor remains single-threaded and dumb:

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

- `permission_plan.actions` contain only runtime permission and app-op entries,
  and they execute only when a `grant_permissions` step runs
- there is no longer a global post-step permission phase
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
11. `permissions`
12. `steps`

Current ordering rules inside canonical recipe YAML:

- `inputs` preserve authored insertion order
- `artifacts` preserve authored insertion order
- `artifact_groups` preserve editor-managed order
- artifact-group membership lists preserve authored/editor-managed order
- UI list sorting for inputs or artifacts is view-only and does not redefine authored order

## Current Known Gaps / Follow-up

Known intentional gaps:

- recipe section editing widgets are not implemented yet
- executor remains single-threaded
- artifact download uses Python stdlib networking only
- archive extraction is still ZIP-oriented in practice
- `grant_permissions` policy metadata is still relatively minimal
- app-private write ownership/uid remapping is not implemented yet
- current CLI bind ids are still normalized internal-style ids rather than a
  cleaner authored ref syntax

If future work changes any of the above, update this document in the same change.
