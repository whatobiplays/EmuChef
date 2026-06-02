# PySide6 Retirement / Quarantine Plan

## Purpose

This document records the PySide6 retirement and quarantine plan for EmuChef.
It is a planning and classification document only.

This document does not retire PySide6, change entrypoints, remove dependencies,
move source files, quarantine tests, change the Tauri runtime, or certify full
Tauri feature parity.

## Current Status

- `pyproject.toml` declares `PySide6>=6.8` only in the optional
  `pyside-editor` extra. The base Python dependencies do not include PySide6.
- `pyproject.toml` still publishes the `emuchef-editor` script entrypoint, and
  that script points at `emuchef_editor.app.app:main`.
- `src/emuchef_editor/app/app.py` is the current `emuchef-editor` launch path.
  It imports `PySide6.QtWidgets.QApplication` at runtime, writes an error to
  stderr, and returns exit code `1` when PySide6 is unavailable.
- The active Tauri editor runtime is Rust-sidecar-only. The current ownership
  docs state that the Tauri config editor has no Python fallback, backend
  selector, backend toggle, environment-variable backend choice, or protocol
  negotiation path.
- `src/emuchef_editor/core` and `src/emuchef_editor/api` remain Python
  editor-core and editor-API reference surfaces. They support comparison,
  developer, golden-generation, and legacy workflows. They are not retired by
  PySide6 retirement, and they are not part of the packaged Tauri editor
  runtime.
- Python CLI, planner, executor, and golden-generation tooling remain separate
  Python-owned or Python-reference surfaces unless later work explicitly ports,
  retains, or retires them.

## Non-Goals

This task does not:

1. Remove PySide6 from packaging.
2. Change or remove `emuchef-editor`.
3. Delete, move, or rename PySide source files.
4. Port, quarantine, delete, or rename tests.
5. Claim Tauri feature parity is complete.
6. Retire Python editor API, editor core, golden-generation, or reference
   tooling.
7. Change Rust, Tauri, frontend, package, schema, or runtime behavior.

## PySide-Only / PySide-Coupled Behavior

The following behavior is PySide-only or PySide-coupled today:

- The Qt desktop shell in `src/emuchef_editor/app`, including
  `MainWindow`, menus, toolbars, splitters, tabs, status messages, and close
  handling.
- Workspace list and open-workspace behavior in the PySide shell, including
  keyboard activation, double-click handling, workspace refresh, and template
  listing.
- Qt dialogs, layouts, form controls, and tooltips used by the PySide editor
  pages.
- PySide Save and Save As UI behavior, including file dialogs, overwrite
  confirmation, dirty state display, and undo/redo state refresh.
- Unsaved-change prompts before opening another recipe, starting a new recipe,
  or closing the window.
- New Recipe from `templates/authored/`, including the PySide dialog that
  collects template, recipe id, destination directory, and output filename.
- Read-only template preview in the PySide New Recipe dialog.
- Workspace template discovery and refresh behavior that shows recipe templates
  separately from authored recipes.

Create-from-template requires an explicit port/retire/no-parity decision before
PySide deletion. If accepted ADR/docs keep it as developer/reference tooling,
full PySide deletion must either port the workflow to Tauri, retire it, or move
it to non-PySide tooling.

## Related Documents

- [ADR 0001: Rust Tauri Editor Runtime Ownership](docs/adr/0001-rust-tauri-editor-runtime-ownership.md)
- [Rust Backend Cutover Readiness Audit](docs/rust-backend-cutover-readiness.md)
- [Rust CLI and Executor Parity Strategy](docs/rust-cli-executor-parity.md)
- [Tauri Packaging Readiness](docs/release/tauri-packaging-readiness.md)
- [EmuChef Config Editor README](apps/config-editor/README.md)

## Retirement Criteria

These criteria must be rechecked during the implementation phase that actually
retires or quarantines PySide6. Existing Tauri evidence does not mean PySide6 is
retired.

- [ ] Tauri can open existing recipes. Evidence exists; reverify during
  retirement.
- [ ] Tauri can save canonical YAML. Evidence exists; reverify during
  retirement.
- [ ] Tauri can Save As canonical YAML. Evidence exists; reverify during
  retirement.
- [ ] Tauri can select and clear authored root. Evidence exists; reverify during
  retirement.
- [ ] Tauri can edit non-step sections. Evidence exists; reverify during
  retirement.
- [ ] Tauri can edit step params. Evidence exists; reverify during retirement.
- [ ] Tauri can edit step dependencies. Evidence exists; reverify during
  retirement.
- [ ] Tauri can show diagnostics. Evidence exists; reverify during retirement.
- [ ] Tauri can show ref index and ref picker behavior. Evidence exists;
  reverify during retirement.
- [ ] Create-from-template has an explicit port/retire/no-parity decision.
- [ ] PySide-dependent tests are ported, quarantined, or removed.
- [ ] The `emuchef-editor` strategy is implemented.
- [ ] Packaging and release docs are updated after entrypoint decision
  execution.

## Blockers

These blockers must be resolved before deleting PySide6 or the PySide app code:

1. Create-from-template requires an explicit port/retire/no-parity decision
   before PySide deletion. If accepted ADR/docs keep it as developer/reference
   tooling, full PySide deletion must either port the workflow to Tauri, retire
   it, or move it to non-PySide tooling.
2. PySide-dependent tests must be ported, quarantined, or removed in a scoped
   follow-up task.
3. The `emuchef-editor` entrypoint strategy must be implemented in a scoped
   follow-up task.
4. Package and release documentation must be updated after the entrypoint
   strategy is executed.
5. Python editor API, editor core, CLI, planner, executor, and golden-generation
   ownership must remain separately classified. PySide deletion must not imply
   Python deletion.
6. Any retained developer/reference workflow that currently depends on PySide
   must either move to Tauri, move to non-PySide tooling, remain intentionally
   quarantined, or be explicitly retired.

## Entrypoint Strategy

Current:

- `emuchef-editor` launches the PySide app through
  `src/emuchef_editor/app/app.py`.

Desired future:

- The packaged Tauri app is the editor path.
- The Python `emuchef-editor` script is no longer published, or it is replaced
  with an explicit documented strategy.

Required before changing it:

1. The Tauri template creation decision is executed.
2. PySide test quarantine, ports, or removals are complete.
3. Package and release docs are updated.
4. An explicit scoped implementation task is approved.

No entrypoint changes are made by this document.

## Test Classification

| Test / Area | PySide6 Required? | Classification | Future Action |
| --- | --- | --- | --- |
| `tests/test_editor_app.py` | Yes | Direct Qt app/widget coverage for the PySide app, including Save As, templates, workspace open paths, dialogs, tooltips, and close prompts. | Port, quarantine, or delete later after replacement evidence exists. |
| `tests/test_editor_tooltips.py` | Indirectly coupled today | Imports `emuchef_editor.app.recipe_editor`; that package initializer imports Qt widget modules through `editor.py` and `new_recipe_dialog.py`. | Port metadata import path away from the Qt package or quarantine. |
| `tests/test_step_plugins.py` | Indirectly coupled today | Imports editor metadata through `emuchef_editor.app.recipe_editor`, which couples the test to the Qt package initializer. | Move or port editor metadata to a PySide-free path, or quarantine. |
| `tests/test_editor_core.py` | No direct PySide requirement | Exercises editor core, workspace service, canonical YAML, ref index, validation, Save As, and template document creation. It imports the PySide-free workspace service under `emuchef_editor.app.workspace`. | Keep selected core, workspace, and template coverage. |
| `tests/test_editor_api_*.py` | No | Exercises Python editor API, DTOs, command codec, session manager, server, sidecar, and step-spec DTO behavior. | Keep until Python API and golden tooling are retired or replaced. |
| `tests/test_step_specs_fixture_tool.py` | No | Verifies StepSpec fixture generation and asserts generation does not load PySide or the app package. | Keep as golden/reference guard. |
| `tests/test_templates.py` | No | Verifies templates stay outside authored loading and match current schema shape. | Keep until the template workflow decision is executed. |
| `tests/test_cli.py`, `tests/test_validation.py`, `tests/test_planner_core.py`, `tests/test_executor_core.py` | No | Python CLI, validation, planner, and executor reference coverage. | Keep as Python CLI/planner/executor/reference tests. |
| `apps/config-editor/tests/*.ts` | No | Frontend logic coverage for Tauri editor behavior. | Keep and expand as Tauri retirement evidence. |
| Rust sidecar and Tauri Rust tests | No | Rust backend and Tauri bridge coverage for sidecar runtime, protocol, Save As, authored root, diagnostics, ref index, commands, packaging, and no-Python-runtime checks. | Keep and expand as Tauri retirement evidence. |

## Future Work

1. Execute the Tauri/create-from-template port, retire, or no-parity decision.
2. Port or quarantine PySide-dependent and PySide-coupled tests.
3. Implement the `emuchef-editor` entrypoint migration strategy.
4. Remove or quarantine PySide package/dependency declarations only after the
   entrypoint and test strategy is implemented.
5. Update package, release, and readiness docs after entrypoint and dependency
   cleanup decisions are executed.
