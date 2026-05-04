# Config Editor Plan

## 1. Overview

EmuChef is currently a CLI-first Android handheld provisioner. The current repo is intentionally split across authored I/O, planning, execution, and typed domain models. The config editor is the first UI app in the codebase and is intended to cover **authoring only**, not planning or execution.

The editor should provide a structured UI for authoring EmuChef config files, starting with recipe files. It should eliminate routine hand-editing of YAML while preserving the authored model and reusing existing EmuChef validation and domain logic. The editor must not expose execution-plan internals in the user-facing authoring experience.

## 2. Goals

- Provide a structured UI for authoring EmuChef config files.
- Eliminate routine hand-editing of YAML for recipe authoring.
- Reuse existing EmuChef domain and validation logic wherever possible.
- Keep the editor focused on authored configuration, not runtime behavior.
- Establish a reusable editor foundation that can later be bundled into the final user-facing app.

## 3. Non-Goals

The first release of the editor will not include:

- execution-plan generation UI
- apply/execution UI
- ADB/device interaction
- runtime state inspection
- a generic YAML text editor as a primary workflow
- device-plan or device-profile editing in the first milestone

The editor must operate on authored recipe concepts only and must not leak planner-internal normalized refs or execution details into authored files.

## 4. Initial Scope

Initial scope is **recipe authoring only**.

The editor should work with the current authored recipe model:

- `inputs:` as a map keyed by input id
- `artifacts:` as a map keyed by artifact id
- `artifact_groups:` as a map keyed by group id
- top-level declarative `permissions:`
- ordered `steps:`

Author refs must remain recipe-local in YAML:

- `inputs.<id>`
- `artifacts.<id>.<field>`
- `steps.<id>`
- `steps.<id>.outputs.<field>`

Literal params are authored directly. Only refs use `{ ref: ... }`.

## 5. Product Requirements

### 5.1 Structured editor, not raw YAML editing

The editor is a structured form-based authoring tool. YAML preview is useful and should exist, but it should be read-only in early milestones. The source of truth should be the document model, not arbitrary user-edited YAML text.

### 5.2 Real validation only

Validation shown in the editor must come from the real EmuChef validation path, not a separate hand-rolled validator. Current validation already supports full catalog validation and single-file validation, and current output includes file-level context in `details`, including:

- `file`
- `object_kind`
- `object_id`
- `field` when known

If a clean internal validation API is not available, the implementation must stop and surface that as a design issue for discussion. It must **not** shell out to the CLI.

### 5.3 Canonical YAML output

Saved YAML should be canonical and editor-owned. The editor should not attempt to preserve arbitrary formatting or comments in the initial implementation.

Recommended top-level key order on write:

1. `schema_version`
2. `kind`
3. `id`
4. `name`
5. `description`
6. `provides`
7. `inputs`
8. `artifacts`
9. `artifact_groups`
10. `permissions`
11. `steps`

Explicit step ordering must be preserved.

Refs should be emitted explicitly as `{ ref: ... }`, not shorthand.

### 5.4 Authored model only

The editor must operate on authored YAML concepts only. It must not expose normalized execution-plan refs or runtime-only values in the main authoring surface. The authored/planner boundary is already a core design rule in EmuChef and should remain intact.

## 6. Architecture

The repo already has a clean split:

- `src/emuchef/io`: authored loading, validation, YAML I/O
- `src/emuchef/planner`: normalization, draft/session logic, execution-plan emission
- `src/emuchef/executor`: runtime ref resolution, artifact handling, ADB, step execution
- `src/emuchef/domain`: typed models and enums

The editor should layer on top of this without changing planner or executor behavior.

### 6.1 Editor package structure

```text
src/
  emuchef/
    domain/
    io/
    planner/
    executor/

  emuchef_editor/
    core/
      documents/
      validation/
      refs/
      templates/
      yaml/
      services/
    app/
      app.py
      main_window.py
      workspace/
      recipe_editor/
      shared/
```

### 6.2 Architecture boundaries

- `emuchef_editor.core` may import `emuchef.*`
- `emuchef_editor.app` may import `emuchef_editor.core` and `emuchef.*`, but should prefer going through `core`
- validation, YAML writing, ref indexing, and rename/update logic must live in `core`
- widget code must not directly own schema logic or raw YAML serialization
- widget code must not treat raw YAML dicts as the source of truth

## 7. Milestones

### M1: Foundation and shell

Goal: establish the editor package, document model, YAML round-trip, validation bridge, ref index scaffolding, and bare desktop shell.

Deliverables:

- `src/emuchef_editor/` package skeleton
- `core/yaml` loader and canonical writer
- `core/validation` adapter wired to the real EmuChef validation path
- `core/documents/RecipeDocument`
- `core/refs/RefIndex` first pass
- `app` shell with:
  - workspace/file panel
  - active document area
  - diagnostics pane
  - YAML preview pane
- open an existing recipe file into a `RecipeDocument`
- save a document back to disk

Non-goals:

- no real section editing widgets yet
- no templates flow yet
- no step editor yet
- no rename/delete safety flows yet

### M2: Core recipe sections

Goal: edit simple recipe sections safely.

Deliverables:

- Overview page
- Inputs page
- Artifacts page
- Artifact groups page
- dirty tracking
- live diagnostics refresh
- YAML preview refresh

### M3: Creation workflows

Goal: make recipe creation practical.

Deliverables:

- Permissions page
- “New from template” flow using `templates/authored/`
- workspace support for templates vs authored files
- basic create/save-as flow

Templates already exist outside `authored/` specifically so the loader does not treat them as real authored inputs. The editor should build on that.

### M4: Step authoring

Goal: support real recipe authoring.

Deliverables:

- ordered step list
- add/remove/duplicate/reorder
- step-specific forms for current supported step types
- dependency picker
- ref picker
- inline validation/help for common step types

Current supported step types are:

- `resolve_artifacts`
- `extract_artifacts`
- `extract_archive`
- `copy_files`
- `install_apk`
- `grant_permissions`
- `launch_app`
- `wait`
- `force_stop_app`

### M5: Safety and polish

Goal: make the editor trustworthy for day-to-day use.

Deliverables:

- rename with local ref updates
- delete-with-usage-check
- find usages
- workspace status badges
- unsaved changes prompts
- keyboard shortcuts
- small UX polish

## 8. Milestone 1 Design Requirements

### 8.1 Document abstraction

Milestone 1 should introduce a real document object, not raw dict juggling.

Suggested minimum shape:

```python
class RecipeDocument:
    path: Path | None
    is_dirty: bool

    def get_model(self) -> RecipeDefinition: ...
    def replace_model(self, model: RecipeDefinition) -> None: ...
    def validate(self) -> list[Diagnostic]: ...
    def build_ref_index(self) -> RefIndex: ...
    def to_yaml(self) -> str: ...
    def save(self, path: Path | None = None) -> None: ...
```

### 8.2 Ref indexing

Milestone 1 should introduce a first-pass `RefIndex` that can at least enumerate:

- inputs
- artifacts
- steps
- step outputs when discoverable from current step metadata/specs

This is scaffolding for later ref pickers, rename safety, usage checks, and step editing.

### 8.3 Validation behavior

At minimum:

- validate on open
- validate on save
- show grouped diagnostics in the right pane
- expose file/object/field details from validator output when available

### 8.4 Desktop shell

Milestone 1 should establish the main layout direction:

- left: workspace/file panel
- center: active document area
- right: tabs for Diagnostics and YAML Preview

The center pane can show placeholder recipe editor content in M1, but the shell should already prove the workspace/document/inspection model.

## 9. Acceptance Criteria for Milestone 1

Milestone 1 is complete when all of the following are true:

1. The app launches successfully.
2. A user can open a workspace root.
3. The workspace panel can list recipe files from the authored area.
4. Opening a recipe file constructs a `RecipeDocument`.
5. The document can load existing YAML and save canonical YAML back to disk.
6. The Diagnostics pane shows real EmuChef validation results for the open document.
7. The YAML Preview pane shows the current canonical YAML for the open document.
8. The code is cleanly separated between `core` and `app`.
9. Planner and executor behavior remain unchanged.
10. No shell-out validation path is introduced.

## 10. Acceptance Target

The current RetroArch provisioning recipe is the practical acceptance target for the editor direction because it already exercises most of the authored model:

- remote APK/core artifacts
- artifact groups
- permissions
- ordered steps
- app-private copy behavior
- optional input behavior
- explicit config destination path

The editor does not need to fully edit every part of that recipe in Milestone 1, but the design should clearly support that path in later milestones.

## 11. Risks and Guardrails

### 11.1 Do not preserve arbitrary comments/formatting in M1

That will blow up scope for no real gain.

### 11.2 Do not build widgets before the document foundation

If UI widgets become the source of truth first, the editor architecture will rot fast.

### 11.3 Do not shell out for validation

If the internal validation path is too awkward or missing, that is a repo/API design issue to resolve directly, not something to paper over with CLI invocation.

### 11.4 Do not mix authored and runtime concepts

The editor must stay on the authored side of the current architecture boundary.
