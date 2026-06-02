# PySide6 Removal Status

## Purpose

This document records the current PySide6 removal state for EmuChef. It does not
claim Python deletion, full Rust parity, public release readiness, or removal of
Python CLI/planner/executor/golden tooling.

## Current Invariant

A clean default install and normal active runtime/test path do not require,
import, or launch PySide6.

Python remains in the repository for CLI, planner, executor, fixture/golden
generation, and real-device apply until later parity or retirement work
explicitly replaces or removes those surfaces.

## Removed PySide Surfaces

- `pyproject.toml` does not declare PySide6 in base dependencies or optional
  dependencies.
- The default Python package does not publish an `emuchef-editor` console script.
- The legacy PySide source package `src/emuchef_editor/app` is removed.
- The legacy PySide test package `tests/legacy` is removed.
- There is no `pyside-editor` optional dependency extra.

## Supported PySide-Free Surfaces

These modules are supported PySide-free import surfaces and must not import
PySide6, `emuchef_editor.app`, or Qt-specific types:

- `emuchef_editor.core.workspace`
- `emuchef_editor.core.metadata.tooltips`
- `emuchef_editor.core.metadata.step_metadata`

Normal core/API tests import these modules directly.

## Template Creation Disposition

`createRecipeFromTemplate` is implemented by the Rust sidecar backend for
protocol parity. GUI create-from-template is retired from the normal editor
path, and no Tauri UI flow is present unless a future product requirement
reintroduces GUI template creation.

## Test Classification

| Test / Area | PySide6 Required? | Classification |
| --- | --- | --- |
| `tests/test_editor_tooltips.py` | No | PySide-free metadata coverage through `emuchef_editor.core.metadata.tooltips`. |
| `tests/test_step_plugins.py` | No | Step plugin and PySide-free metadata coverage through `emuchef_editor.core.metadata.step_metadata`. |
| `tests/test_editor_core.py` | No | Editor core, workspace, canonical YAML, ref index, validation, Save As, and template document creation coverage. |
| `tests/test_templates.py` | No | Template schema and authored-loader isolation coverage. |
| Rust and Tauri tests | No | Active Rust sidecar and Tauri bridge/runtime coverage. |

Normal Python test discovery or targeted non-legacy editor tests must not import
PySide6, even on machines where PySide6 happens to be installed.

## Guard

`npm run check:no-pyside-runtime` fails on:

- PySide6 in base Python dependencies.
- PySide6 in optional Python dependencies.
- A published `emuchef-editor` console script.
- Active source imports of PySide6.
- Active source imports of `emuchef_editor.app`.
- Normal test imports of PySide6.
- Normal test imports of `emuchef_editor.app`.
- Any Python files under the removed legacy PySide source path
  `src/emuchef_editor/app`.
- Any Python files under the removed legacy PySide test path `tests/legacy`.
- Qt-specific names in supported PySide-free core metadata modules.

The guard allows documentation references that describe removed PySide status.

## Remaining Python Deletion Work

PySide6 removal is not Python deletion. Full Python removal remains blocked by
Python CLI replacement/retirement, planner/executor breadth, real-device apply
ownership, fixture/golden generation ownership, Python tests/docs/scripts
cleanup, and Rust-native ownership for any behavior still proven only by Python
reference tests.
