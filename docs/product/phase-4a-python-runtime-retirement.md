# Phase 4A Python Runtime Retirement

## 1. Contract

Rust is the sole implementation of EmuChef's CLI, authored-data validation,
catalog loading, planning, execution, configuration documents, editor protocol,
and JSONL sidecar. Python is not a product, development, test, packaging, or
release prerequisite. No backend selector, compatibility executable, fallback,
or Python sidecar is supported.

This retirement does not change authored formats, recipe semantics, plan
ordering, execution behavior, CLI flags, protocol DTOs, configuration or
recovery schemas, diagnostics/cache behavior, Tauri authority, or packaging
authority.

## 2. Pre-deletion inventory and classification

### 2.1 Obsolete runtime/reference implementation

The following 70 files formed the frozen `emuchef` reference implementation:

1. `src/emuchef/__init__.py`
1. `src/emuchef/domain/__init__.py`
1. `src/emuchef/domain/_validation.py`
1. `src/emuchef/domain/app_definition.py`
1. `src/emuchef/domain/artifacts.py`
1. `src/emuchef/domain/codes.py`
1. `src/emuchef/domain/constants.py`
1. `src/emuchef/domain/copy_policy.py`
1. `src/emuchef/domain/device_context.py`
1. `src/emuchef/domain/device_plan.py`
1. `src/emuchef/domain/device_profiles.py`
1. `src/emuchef/domain/draft_changes.py`
1. `src/emuchef/domain/draft_plan.py`
1. `src/emuchef/domain/draft_update_result.py`
1. `src/emuchef/domain/execution_plan.py`
1. `src/emuchef/domain/history_entry.py`
1. `src/emuchef/domain/input_declaration.py`
1. `src/emuchef/domain/issues.py`
1. `src/emuchef/domain/param_values.py`
1. `src/emuchef/domain/planning_result.py`
1. `src/emuchef/domain/recipe.py`
1. `src/emuchef/domain/refs.py`
1. `src/emuchef/domain/runtime_state.py`
1. `src/emuchef/domain/step.py`
1. `src/emuchef/domain/step_specs.py`
1. `src/emuchef/domain/step_types.py`
1. `src/emuchef/domain/validation_result.py`
1. `src/emuchef/executor/__init__.py`
1. `src/emuchef/executor/adb.py`
1. `src/emuchef/executor/artifact_io.py`
1. `src/emuchef/executor/conditions.py`
1. `src/emuchef/executor/copy_helpers.py`
1. `src/emuchef/executor/permission_helpers.py`
1. `src/emuchef/executor/resolver.py`
1. `src/emuchef/executor/result.py`
1. `src/emuchef/executor/runner.py`
1. `src/emuchef/executor/runtime_values.py`
1. `src/emuchef/executor/step_runtime.py`
1. `src/emuchef/io/__init__.py`
1. `src/emuchef/io/execution_plan_io.py`
1. `src/emuchef/io/loader.py`
1. `src/emuchef/io/serde.py`
1. `src/emuchef/io/validation.py`
1. `src/emuchef/planner/__init__.py`
1. `src/emuchef/planner/bindings.py`
1. `src/emuchef/planner/catalog.py`
1. `src/emuchef/planner/conflicts.py`
1. `src/emuchef/planner/contracts.py`
1. `src/emuchef/planner/dependencies.py`
1. `src/emuchef/planner/draft_builder.py`
1. `src/emuchef/planner/emitter.py`
1. `src/emuchef/planner/history.py`
1. `src/emuchef/planner/ids.py`
1. `src/emuchef/planner/operations.py`
1. `src/emuchef/planner/profile_matching.py`
1. `src/emuchef/planner/service.py`
1. `src/emuchef/steps/__init__.py`
1. `src/emuchef/steps/builtin.py`
1. `src/emuchef/steps/contracts.py`
1. `src/emuchef/steps/handlers/__init__.py`
1. `src/emuchef/steps/handlers/copy_files.py`
1. `src/emuchef/steps/handlers/extract_archive.py`
1. `src/emuchef/steps/handlers/extract_artifacts.py`
1. `src/emuchef/steps/handlers/force_stop_app.py`
1. `src/emuchef/steps/handlers/grant_permissions.py`
1. `src/emuchef/steps/handlers/install_apk.py`
1. `src/emuchef/steps/handlers/launch_app.py`
1. `src/emuchef/steps/handlers/resolve_artifacts.py`
1. `src/emuchef/steps/handlers/wait.py`
1. `src/emuchef/steps/planner_hooks.py`

The following 19 files formed the frozen editor-core reference implementation:

1. `src/emuchef_editor/__init__.py`
1. `src/emuchef_editor/core/__init__.py`
1. `src/emuchef_editor/core/analysis/__init__.py`
1. `src/emuchef_editor/core/analysis/usages.py`
1. `src/emuchef_editor/core/documents/__init__.py`
1. `src/emuchef_editor/core/documents/commands.py`
1. `src/emuchef_editor/core/documents/history.py`
1. `src/emuchef_editor/core/documents/recipe_document.py`
1. `src/emuchef_editor/core/metadata/__init__.py`
1. `src/emuchef_editor/core/metadata/step_metadata.py`
1. `src/emuchef_editor/core/metadata/tooltips.py`
1. `src/emuchef_editor/core/refs/__init__.py`
1. `src/emuchef_editor/core/refs/ref_index.py`
1. `src/emuchef_editor/core/validation/__init__.py`
1. `src/emuchef_editor/core/validation/validator_service.py`
1. `src/emuchef_editor/core/workspace.py`
1. `src/emuchef_editor/core/yaml/__init__.py`
1. `src/emuchef_editor/core/yaml/loader.py`
1. `src/emuchef_editor/core/yaml/writer.py`

`src/LEGACY_PYTHON.md` described both trees as frozen reference code pending
deletion. It had no continuing purpose after this contract became authoritative.

### 2.2 Obsolete Python tests

The following eight files tested only the deleted reference implementation:

1. `tests/support.py`
1. `tests/test_editor_core.py`
1. `tests/test_editor_tooltips.py`
1. `tests/test_executor_core.py`
1. `tests/test_planner_core.py`
1. `tests/test_step_plugins.py`
1. `tests/test_templates.py`
1. `tests/test_validation.py`

### 2.3 Package metadata and dependencies

`pyproject.toml` declared a setuptools package rooted at `src`, Python 3.11 or
newer, and the sole runtime dependency `PyYAML>=6.0`. It declared no console
entrypoint. No `src/emuchef/__main__.py`, `emuchef.cli`, supported
`python -m emuchef` path, requirements file, Python lockfile, or active Python
fixture generator existed at the start of Phase 4A. The complete project file
was obsolete because no tooling-only Python remains.

### 2.4 Scripts, CI, documentation, and packaging references

The Config Editor previously carried three narrow Node guards and their tests:

1. `apps/config-editor/scripts/check-no-python-runtime.mjs`
2. `apps/config-editor/scripts/check-no-python-runtime.test.mjs`
3. `apps/config-editor/scripts/check-no-python-editor-api.mjs`
4. `apps/config-editor/scripts/check-no-python-editor-api.test.mjs`
5. `apps/config-editor/scripts/check-no-pyside-runtime.mjs`
6. `apps/config-editor/scripts/check-no-pyside-runtime.test.mjs`

Those guards depended on `pyproject.toml` or the frozen `src`/`tests` trees and
were superseded by the repository-wide retirement guard. Their package-script
references were obsolete. The macOS qualification workflow contained no Python
setup or invocation before retirement.

Current-state Python references existed in `README.md`, `CONTEXT.md`,
`docs/architecture/runtime-ownership.md`, and the Config Editor README. They
described code as pending deletion and required replacement with the completed
retirement state. The Rust authored-validation test also contained one comment
naming a deleted Python implementation detail.

Historical ADRs, release evidence, the repository's prior result records,
`docs/testing/compatibility-fixtures.md`, and
`crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1` are
historical or immutable evidence. They are retained unchanged. Existing macOS
bundle inspection commands that search for Python executables, frameworks,
source files, or retired names are active negative packaging controls and are
also retained.

### 2.5 Retained tooling-only Python

None. The tooling-only Python allowlist is empty.

## 3. Rust ownership evidence

| Supported surface | Authoritative evidence |
| --- | --- |
| Authored corpus and catalog validation | `authored_corpus.rs`, `authored_validation.rs`, and `catalog_validation.rs` |
| CLI validate, plan, and apply contracts | `cli_contract.rs`, `planner_contract.rs`, and process-level CLI tests |
| Dry-run and guarded real apply | Rust executor/adapter tests and default-off real-execution checks |
| Configuration documents | `runtime_configuration_contract.rs` and `user_configuration_contract.rs` |
| Editor documents and YAML | editor command/session tests, `yaml_contract.rs`, and frozen compatibility fixtures |
| JSONL sidecar operations | `protocol_contract.rs` and `sidecar_protocol.rs` |
| Config Editor client | `npm --prefix apps/config-editor run check:rust-runtime` |
| End-user client | typecheck, build, logic, security, packaging, and preflight checks |
| Packaged runtime absence | macOS bundle inspection and sidecar packaging tests |

Parity is semantic. Rust canonicalization, stable error codes, accepted and
rejected authored data, documented CLI behavior, and protocol outcomes are
authoritative. Frozen fixtures are compatibility evidence, not a Python oracle,
and are never regenerated as part of retirement.

## 4. Reintroduction guard

`scripts/check-python-runtime-retirement.mjs` inspects tracked and non-ignored
files. It rejects Python source/bytecode, Python project metadata, Python or
`uv` runtime invocations, Python module/sidecar paths, backend selectors,
compatibility flags and environment variables, and retired runtime names.

Historical documentation, immutable fixtures, tests, and explicit negative
bundle-enforcement paths are excluded from runtime-token scanning. They are not
exceptions for executable Python: Python files and project metadata remain
prohibited everywhere because the tooling allowlist is empty.

Both Tauri packages expose `check:python-runtime-retirement`. The Config Editor
`check:rust-runtime`, the end-user security suite, and CI invoke the guard.
Every invocation enforces minimum coverage of 95% for lines, branches, and
functions, then runs the direct repository scan as a separate integration step.
The corrected focused test suite achieves 100% line coverage, 97.92% branch
coverage, and 100% function coverage.

## 5. Acceptance and evidence checklist

1. The 97 Python source/test files, `src/LEGACY_PYTHON.md`, and `pyproject.toml` are absent.
2. No Python tooling exception or dependency remains.
3. Rust remains the sole implementation of every supported runtime surface.
4. Current-state documentation describes Python as retired, not selectable.
5. Historical evidence and frozen fixtures remain unchanged.
6. The retirement guard and its tests pass with at least 95% enforced and
   measured line, branch, and function coverage.
7. Rust, both frontends, security, packaging, and preflight checks pass.
8. Forbidden authored, frontend, Tauri, lockfile, and evidence paths are unchanged.

## 6. Verification

Run from the repository root:

```bash
rtk test node --test --experimental-test-coverage --test-coverage-include=scripts/check-python-runtime-retirement.mjs --test-coverage-lines=95 --test-coverage-functions=95 --test-coverage-branches=95 scripts/check-python-runtime-retirement.test.mjs
rtk proxy node scripts/check-python-runtime-retirement.mjs
rtk cargo fmt --check --manifest-path crates/emuchef-rust-backend/Cargo.toml
rtk cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
rtk cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings
rtk npm --prefix apps/config-editor run check:rust-runtime
rtk npm --prefix apps/emuchef-app run typecheck
rtk npm --prefix apps/emuchef-app run build
rtk npm --prefix apps/emuchef-app run test:logic
rtk npm --prefix apps/emuchef-app run test:security
rtk npm --prefix apps/emuchef-app run test:packaging
rtk npm --prefix apps/emuchef-app run package:macos:preflight
rtk git diff --check
rtk git status --short
```
