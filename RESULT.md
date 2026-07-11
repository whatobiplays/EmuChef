# Rust Cleanup Result

Date: 2026-07-10

The cleanup is complete. Rust is the sole product runtime, Python is frozen
non-executable reference source, compatibility fixtures are immutable, and the
repository no longer presents an active migration architecture.

## 1. Commits

| SHA | Purpose |
| --- | --- |
| `3f3aa99` | Consolidate document operations onto one non-prefixed Tauri command surface and remove transport ids from frontend responses. |
| `ad24a32` | Remove the shadow planner, migration smoke surface, broad dead-code suppressions, and other transitional Rust code; route the CLI through the typed planner runtime. |
| `08f11ce` | Move the 60 byte-identical Python-origin goldens to frozen `compatibility_goldens_v1` evidence and delete live parity/regeneration machinery. |
| `e5230d5` | Delete migration readiness gates, preflight checks, apply-bridge smoke tooling, and their orphan fixtures. |
| `ce3d81b` | Remove Python CLI entrypoints and CLI tests, declare the remaining Python packages frozen, and strengthen the no-Python-runtime guard. |
| `25df9fd` | Rename permanent Rust and frontend tests by behavior and replace migration labels in active code and test names. |
| `df8b9d2` | Replace the historical migration document set with concise current architecture, testing, and release documents. |
| `9c5dade` | Add the Rust real-device RetroArch validation runbook and link it from current documentation. |
| `51d1737` | Remove the unused root npm manifest, lockfile, and `headroom-ai` dependency. |
| `42c292d` | Remove the final migration-phase comment and obsolete sidecar-prefixed test names found by the repository audit. |

This report is committed after the listed implementation commits. Its own SHA
cannot be embedded without rewriting the commit and is reported in the task
completion message.

## 2. Tracked Files Deleted

Runtime and entrypoint deletion:

- `crates/emuchef-rust-backend/src/bin/emuchef-plan-shadow.rs`
- `crates/emuchef-rust-backend/src/plan_shadow.rs`
- `src/emuchef/__main__.py`
- `src/emuchef/cli.py`
- `src/emuchef/planner_cli.py`
- root `package.json` and `package-lock.json`

Migration/parity/readiness tooling deletion:

- `tools/compare_rust_python_plan.py` and
  `tools/plan_parity_scenarios.json`
- `tools/check_python_planner_deletion_preflight.py`
- `tools/check_rust_planner_cutover_readiness.py`
- `tools/smoke_launcher_injected_planner.py`
- `tools/smoke_rust_apply_dry_run_bridge.py`
- all detected-facts, experimental/live-probe, mismatch-warning, and shadow
  smoke tools under `tools/`
- the corresponding Python test modules under `tests/`
- `tests/test_cli.py`
- `tests/fixtures/apply_dry_run/minimal_execution_plan.yaml`
- `tests/fixtures/readiness/current_static_readiness_report.json`
- `apps/config-editor/scripts/check-no-python-fixture-regeneration.mjs` and its
  test

Rust test deletion:

- `crates/emuchef-rust-backend/tests/phase7m_plan_shadow.rs`
- `crates/emuchef-rust-backend/tests/phase8r_detected_facts_shadow.rs`
- the old `phase6y_step_specs_native.rs`, whose behavior-focused coverage now
  lives in `step_specs_contract.rs`

Documentation deletion:

- `config-editor-plan.md`
- ADRs 0004 and 0005
- the old real-device matrix and Tauri packaging-readiness document
- Python retirement, fixture ownership, and deletion-preflight plans
- all top-level Rust cutover, parity, readiness, resolver, packaged-path,
  launcher-smoke, probe, mismatch, and migration maintenance documents

## 3. Files Renamed

- `apps/config-editor/src/components/phase5EditorState.logic.ts` to
  `editorState.logic.ts`
- `apps/config-editor/tests/phase5EditorState.logic.test.ts` to
  `editorState.logic.test.ts`
- Rust integration suites:
  - `phase6e_yaml.rs` to `yaml_contract.rs`
  - `phase6f_sessions.rs` to `editor_sessions.rs`
  - `phase6g_commands.rs` to `editor_commands.rs`
  - `phase6h_ref_index.rs` to `ref_index_contract.rs`
  - `phase6i_non_step_commands.rs` to `editor_collection_commands.rs`
  - `phase6j1_step_commands.rs` to `editor_step_commands.rs`
  - `phase6j2_step_internals.rs` to `editor_step_internals.rs`
  - `phase6k_validation.rs` to `authored_validation.rs`
  - `phase6l_catalog_validation.rs` to `catalog_validation.rs`
  - `phase6m_planner.rs` to `planner_contract.rs`
  - `phase6o_protocol.rs` to `sidecar_protocol.rs`
  - `phase6s_cli.rs` to `cli_contract.rs`
  - `phase6t_authored_corpus.rs` to `authored_corpus.rs`
  - `protocol_skeleton.rs` to `protocol_contract.rs`
- `tests/fixtures/python_goldens/` to
  `tests/fixtures/compatibility_goldens_v1/` inside the Rust crate. All 60
  fixture files retained identical SHA-256 digests. Their historical filenames
  and persisted values were intentionally not rewritten.

## 4. Retained Python Surface

`src/emuchef/`, `src/emuchef_editor/core/`, and their remaining reference tests
are retained temporarily because this cleanup authorized deletion only when a
surface was proven migration-only. They contain domain, planner, executor,
serialization, validation, and editor-core reference code that was not required
to be fully deleted here. `src/LEGACY_PYTHON.md` freezes the boundary.

The retained packages have no console script, `__main__.py`, product command,
alternate backend selector, fixture generator, or editor runtime. Running
`PYTHONPATH=src python3 -m emuchef` fails because the package has no executable
module. No new Python functionality or expected Rust output was produced.

## 5. Tauri and Rust Consolidation

Removed document aliases include `sidecar_list_step_specs`,
`sidecar_open_recipe`, `sidecar_get_document`,
`sidecar_apply_recipe_command`, `sidecar_undo`, `sidecar_redo`,
`sidecar_save_recipe`, `sidecar_save_recipe_as`, `sidecar_validate`,
`sidecar_emit_yaml`, `sidecar_get_ref_index`, and
`sidecar_set_document_authored_root`. Lifecycle commands retain `sidecar_`
because they manage the process.

The shadow binary/module and shadow-only tests were deleted. Active planning is
owned by `src/planner_runtime.rs`, which is called directly by the product CLI.
Broad dead-code suppressions were removed; test-only helpers are narrowly
compiled for tests, unused code was deleted, and clippy is warning-free. No
requested deletion had to be reversed to preserve active Rust or Tauri product
behavior.

## 6. Documentation Consolidation

The current documentation set is centered on:

- `README.md` and `CONTEXT.md`
- `docs/architecture/runtime-ownership.md`
- `docs/architecture/editor-runtime.md`
- `docs/architecture/planner-executor.md`
- `docs/testing/compatibility-fixtures.md`
- `docs/release/release-readiness.md`
- `docs/manual/packaged-gui-e2e.md`
- `docs/manual/real-device-retroarch-validation.md`
- present-tense ADRs 0001 through 0003

A repository-wide local Markdown link audit passed across 14 current files.

## 7. Generated Local Artifacts Deleted

The final filesystem-only cleanup removed:

- `.emuchef_cache/` and `.emuchef_runtime/`
- both Rust `target/` trees
- generated editor sidecar binaries and metadata, preserving
  `apps/config-editor/src-tauri/binaries/.gitignore`
- `apps/config-editor/dist/` and `apps/config-editor/src-tauri/gen/`
- `src/emuchef.egg-info/`
- the orphaned root `node_modules/`
- repository `__pycache__/` directories and `.DS_Store` files

`.codegraph`, `.venv`, app-local `node_modules`, app-local lockfiles, source
fixtures, compatibility goldens, authored recipes, and maintained documents
were preserved. No tracked commit contains the ignored-artifact deletion.

## 8. Verification

The complete matrix passed after the last implementation commit and before the
generated-artifact cleanup:

| Command | Result |
| --- | --- |
| `cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check` | Passed |
| `cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml` | Passed |
| `cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml` | 329 passed, 7 ignored |
| `cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets -- -D warnings` | Passed, no issues |
| `npm run check:rust-runtime` from `apps/config-editor` | Passed, including packaging and no-Python guards |
| `npm run typecheck` from `apps/config-editor` | Passed |
| `npm run test:logic` from `apps/config-editor` | 94 passed |
| `npm run build` from `apps/config-editor` | Passed |
| `cargo fmt --manifest-path apps/config-editor/src-tauri/Cargo.toml --all -- --check` | Passed |
| `cargo check --manifest-path apps/config-editor/src-tauri/Cargo.toml` | Passed |
| `cargo test --manifest-path apps/config-editor/src-tauri/Cargo.toml` | 29 passed |
| `cargo clippy --manifest-path apps/config-editor/src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed, no issues |

Focused verification also passed for every preceding commit. The frozen fixture
move was verified with matching SHA-256 manifests for all 60 files. The retained
Python validation reference tests passed with the repository virtual
environment after using `PYTHONPATH=src:tests` (12 tests). The explicit Rust
RetroArch recipe validation and plan commands used in the manual runbook passed.

Repository searches found no removed Tauri aliases, `python_goldens`, Python
comparison tool, Python planner backend, shadow executable/module, migration
comments, or migration-language current docs. `PySide6` remains only in
negative guard/test assertions. Phase-labelled strings remain only where they
are fixture names, frozen persisted values, or assertions against those values.

## 9. Tests Not Run

- Real-device ADB apply was not run automatically because it is destructive and
  requires a specifically authorized test device and artifact set.
- Real packaged GUI validation was not run because it requires installation and
  observation on each supported release target.
- HTTP(S) artifact tests were not run because network artifact downloading is
  not implemented.
- The complete frozen Python legacy suite was not treated as product
  verification; only the focused retained validation reference tests were run.

## 10. Remaining Blockers and Next Feature

There is no known blocker to this cleanup result. Public release readiness is
still blocked on:

- HTTP(S) artifact downloading;
- recorded real-device RetroArch evidence;
- recorded packaged GUI evidence on supported targets;
- signing and macOS notarization decisions and automation;
- updater support;
- CSP hardening; and
- cross-platform release automation and artifact inspection.

HTTP(S) artifact downloading was deliberately not implemented. It is the next
product feature and must include strict TLS validation, bounded redirects and
timeouts, deterministic cache keys, temporary files, atomic publication,
partial-file cleanup, typed failures, local-server tests, and clean-cache
RetroArch validation.

The exact manual runbook path is:

`docs/manual/real-device-retroarch-validation.md`
