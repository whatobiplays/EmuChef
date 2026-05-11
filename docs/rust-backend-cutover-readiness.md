# Rust Backend Cutover Readiness Audit

Phase 6T audits the experimental Rust backend against the Python reference and
records what still blocks a hard cutover. It does not start the cutover.

## Executive Summary

The Rust backend has strong crate-local, fixture-backed parity for the editor
protocol surface implemented through Phase 6S. Phase 6U addresses the 6T
Tauri-local cutover blockers by implementing Rust sidecar `saveRecipeAs`, making
the Rust JSONL sidecar interactive, and hard-wiring the Tauri editor runtime to
launch the Rust sidecar with no Python fallback or backend selector. Phase 6V
adds host-target Tauri v2 `externalBin` packaging for the Rust sidecar and proves
the macOS app bundle includes a launchable Rust backend binary. The Rust backend
is still not a Python CLI replacement and is not broad enough to justify deleting
Python or PySide6.

Verdict: **6U and host-target 6V sidecar packaging are complete for the Tauri
editor path**. This does not mean Python deletion, signing/notarization, updater
support, cross-platform release automation, or public release hardening are
complete.

## 6U Entry Criteria

Phase 6U may start when:

- Rust crate tests pass.
- Tauri Rust sanity tests pass.
- Rust required capabilities satisfy the Tauri compatibility gate.
- The `saveRecipeAs` disposition is decided for 6U.
- StepSpec and editor-protocol goldens are regenerated, or the committed
  fixtures are explicitly recorded as authoritative for 6U.
- 6U remains limited to Tauri hard integration and does not include packaging,
  backend selectors/toggles, Python deletion, or broad parity expansion.

## Current Rust Backend Status

`crates/emuchef-rust-backend` is a standalone experimental crate. It supports
one-shot JSON requests, JSONL sidecar requests, editor document sessions,
fixture-backed StepSpec DTOs, authored YAML load/emit/validation, command
history, RefIndex, internal planner and executor scaffolding, selected fake and
manual real-ADB foundations, and a crate-local CLI subset.

The Tauri editor runtime now uses the Rust sidecar for local/dev and host-target
packaged editor backend requests. Python remains in the repo and remains the
reference for broader CLI/planner/executor behavior until later replacement work
is confirmed.

## Hard Cutover Policy

There is no backend selector, backend toggle, environment variable, config
option, UI switch, or protocol negotiation path. Rust must replace Python through
a hard cutover only after parity is confirmed. If a hard cutover fails after
release, rollback means reverting the cutover commit/package; rollback is not a
runtime option.

## Audited Areas

- Protocol/sidecar: `hello`, capability lists, protocol version, envelopes,
  JSONL lifecycle, stdout/stderr split, unknown request handling, one-shot and
  sidecar behavior.
- Step specs: built-ins, DTO shape, defaults, ref filters, output metadata, and
  embedded Python fixture freshness.
- Authored YAML: load/emit behavior, canonical ordering, refs, unknown fields,
  null/default handling, and top-level permissions rejection.
- Sessions/documents: open/get/save/close, document DTO, dirty/undo/redo,
  diagnostics refresh, authoredRoot persistence, unknown document behavior.
- Commands/history: current external editor command decoders, undo/redo, no-op
  behavior, snapshot history, cleanup, and error-code classification.
- RefIndex: document DTO refIndex, `getRefIndex`, candidates, ordering, source
  kinds, and intentionally unsupported refs.
- Validation: path/session validation, editor-local validation, authoredRoot
  validation, StepSpec contracts, refs, dependencies, cycles, and diagnostics.
- Planner: internal planning result shape, dependency expansion, namespacing,
  bindings/defaults, artifacts, groups, and error/status behavior.
- Executor: internal run result shape, dry-run behavior, filesystem/artifact
  safety, fake-device/DryRunAdb behavior, and real-ADB manual foundations.
- CLI: Python CLI inventory, Rust crate-local subset, validate/apply dry-run,
  stdout/stderr/exit code behavior, and legacy dispatch preservation.
- Tauri integration, packaging/distribution, Python/PySide6 removal, and CI.

## Capability Comparison

| Capability | Python reports | Rust reports | Tauri/Frontend usage | Classification | Cutover impact |
| --- | --- | --- | --- | --- | --- |
| `listStepSpecs` | Required in `src/emuchef_editor/api/protocol.py` | Yes in `crates/emuchef-rust-backend/src/protocol.rs` | Required by `apps/config-editor/src-tauri/src/sidecar_client.rs`; called by React startup | Ready | Required before cutover; covered. |
| `openRecipe` | Required | Yes | Required by sidecar gate and React open flow | Ready | Required before cutover; covered. |
| `getDocument` | Required | Yes | Required by sidecar gate | Ready | Required before cutover; covered. |
| `applyRecipeCommand` | Required | Yes | Required by sidecar gate and editor mutations | Ready with fixture-scoped coverage | Required before cutover; covered for current command surface. |
| `undo` / `redo` | Required | Yes | Required by sidecar gate and UI/menu actions | Ready with fixture-scoped coverage | Required before cutover; covered. |
| `saveRecipe` | Required | Yes | Required by sidecar gate and Save action | Ready with fixture-scoped coverage | Required before cutover; covered. |
| `validate` / `emitYaml` | Required | Yes | Required by sidecar gate and editor actions | Ready with fixture-scoped coverage | Required before cutover; covered. |
| `getRefIndex` | Required | Yes | Required by sidecar gate and document DTO assumptions | Ready with fixture-scoped coverage | Required before cutover; covered. |
| `closeDocument` | Optional in Python | Yes | No current Tauri command or React caller found | Ready | No cutover blocker. |
| `createRecipeFromTemplate` | Optional in Python | No | No Tauri/TypeScript/React caller found; Python sidecar tests and PySide template creation exist | Acceptable deferred gap for Tauri; Python deletion blocker | Not required for Tauri cutover unless a new Tauri create-from-template flow is added. Required before deleting Python/PySide6 template behavior. Do not implement `createRecipeFromTemplate` in 6U unless Tauri integration discovers an active frontend/menu caller. |
| `saveRecipeAs` | Optional in Python | Yes | `sidecar_save_recipe_as` is registered in Tauri and exported from `editorApi.ts`; no current React caller or menu item found | Ready for Tauri cutover | Phase 6U implements Rust sidecar Save As and requires the capability in Tauri because the command is registered and must not forward to an unsupported backend. |

## Parity Status Table

| Area | Python source(s) | Rust source/test(s) | Status | Evidence | Remaining gap | Gap classification | Cutover impact |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Protocol / sidecar | `src/emuchef_editor/api/protocol.py`, `src/emuchef_editor/api/server.py`, `src/emuchef_editor/api/sidecar.py`, `tests/test_editor_api_server.py`, `tests/test_editor_api_sidecar.py` | `crates/emuchef-rust-backend/src/protocol.rs`, `crates/emuchef-rust-backend/src/request.rs`, `crates/emuchef-rust-backend/src/jsonl.rs`, `crates/emuchef-rust-backend/tests/protocol_skeleton.rs`, `crates/emuchef-rust-backend/tests/phase6s_cli.rs` | Partial | Rust and Python both use `protocolVersion: 1`; Rust tests cover envelopes, interactive JSONL line flushing, stdout-only success/error envelopes, malformed JSON, unknown requests, one-shot vs sidecar dispatch, and `saveRecipeAs`; one-shot hello smoke passed. | Rust omits Python optional `createRecipeFromTemplate`; GUI packaged E2E and cross-platform release packaging remain deferred. | `Python deletion blocker`, `release confidence risk` | 6U resolves the registered `saveRecipeAs` Tauri cutover blocker; 6V resolves host-target sidecar packaging without changing Python deletion status. |
| Step specs | `src/emuchef/domain/step_specs.py`, `src/emuchef_editor/api/dto.py`, `tests/test_editor_api_step_specs.py` | `crates/emuchef-rust-backend/src/step_specs.rs`, `crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json`, `crates/emuchef-rust-backend/tests/protocol_skeleton.rs` | Ready with fixture-scoped coverage | Rust embeds the Python-generated fixture; tests assert nine built-ins, DTO shape, defaults/refFilters/output metadata; one-shot `listStepSpecs` smoke passed. | Fixture can drift if Python StepSpecs change and the JSON fixture is not regenerated. | `release confidence risk` | Not a cutover blocker if refreshed before 6U/6V verification. |
| Authored YAML | `src/emuchef_editor/core/yaml/loader.py`, `src/emuchef_editor/core/yaml/writer.py`, `src/emuchef_editor/core/documents/recipe_document.py`, `tests/test_editor_core.py` | `crates/emuchef-rust-backend/src/yaml.rs`, `crates/emuchef-rust-backend/tests/phase6e_yaml.rs`, `crates/emuchef-rust-backend/tests/fixtures/python_goldens/*.emit.yaml`, `*.validate.json` | Ready with fixture-scoped coverage | Tests cover minimal byte-for-byte emit, representative semantic emit, top-level refs, malformed YAML, unsupported steps, and top-level permissions rejection; one-shot emit/validate smokes passed. | Semantic parity is broader than byte-for-byte parity; coverage is fixture-scoped. | `release confidence risk` | Needs broader authored corpus before public release confidence, but not a 6U start blocker. |
| Sessions / documents | `src/emuchef_editor/api/session.py`, `src/emuchef_editor/core/documents/recipe_document.py`, `tests/test_editor_api_sidecar.py` | `crates/emuchef-rust-backend/src/session.rs`, `crates/emuchef-rust-backend/src/document.rs`, `crates/emuchef-rust-backend/src/dto.rs`, `crates/emuchef-rust-backend/tests/phase6f_sessions.rs`, `crates/emuchef-rust-backend/tests/phase6l_catalog_validation.rs` | Partial | Rust covers open/get/save/saveAs/close, DTO shape, dirty/canUndo/canRedo, validation refresh, authoredRoot persistence, unknown documents, missing-parent Save As failure behavior, and sidecar-only session APIs. | No Rust `createRecipeFromTemplate`; Rust document IDs are deterministic test-local IDs rather than UUIDs, which appears protocol-compatible for Tauri but remains a release-confidence consideration. | `Python deletion blocker`, `release confidence risk` | 6U resolves the registered `saveRecipeAs` Tauri cutover blocker. |
| Commands / history | `src/emuchef_editor/api/command_codec.py`, `src/emuchef_editor/core/documents/commands.py`, `src/emuchef_editor/core/documents/history.py` | `crates/emuchef-rust-backend/src/commands.rs`, `crates/emuchef-rust-backend/src/document.rs`, `crates/emuchef-rust-backend/tests/phase6g_commands.rs`, `crates/emuchef-rust-backend/tests/phase6i_non_step_commands.rs`, `crates/emuchef-rust-backend/tests/phase6j1_step_commands.rs`, `crates/emuchef-rust-backend/tests/phase6j2_step_internals.rs` | Ready with fixture-scoped coverage | Tests cover current external command inventory, invalid_command vs command_failed behavior, no-op behavior, cleanup, undo/redo snapshots, save baseline, and Python golden results. | Future command additions in Python must be mirrored; not proven outside current fixture set. | `release confidence risk` | Suitable to start 6U; broaden regression coverage before public release. |
| RefIndex | `src/emuchef_editor/core/refs/ref_index.py`, `src/emuchef_editor/api/dto.py`, `tests/test_editor_api_dto.py` | `crates/emuchef-rust-backend/src/ref_index.rs`, `crates/emuchef-rust-backend/tests/phase6h_ref_index.rs`, `crates/emuchef-rust-backend/tests/phase6i_non_step_commands.rs` | Ready with fixture-scoped coverage | Tests cover document DTO refIndex, `getRefIndex`, inputs, artifacts, steps, outputs, source kinds, ordering, and update after mutations/undo/redo/save. | Intentionally does not index authored param refs, artifact groups, planner state, catalog context, or executor/device data. | `accepted limitation`, `release confidence risk` | Accepted for current editor cutover if frontend assumptions remain unchanged. |
| Validation | `src/emuchef/io/validation.py`, `src/emuchef_editor/core/validation/validator_service.py`, `src/emuchef/planner/contracts.py`, `tests/test_editor_core.py` | `crates/emuchef-rust-backend/src/validation.rs`, `crates/emuchef-rust-backend/src/catalog.rs`, `crates/emuchef-rust-backend/tests/phase6k_validation.rs`, `crates/emuchef-rust-backend/tests/phase6l_catalog_validation.rs` | Ready with fixture-scoped coverage | Tests cover path/session validation, editor-local diagnostics, authoredRoot/catalog diagnostics, StepSpec contract validation, refs, dependencies, cycles, and diagnostics refresh. | Message text is semantically matched for selected fixtures; serde_yaml vs PyYAML differences remain possible. | `release confidence risk` | Needs broader authoredRoot project coverage before release confidence. |
| Planner | `src/emuchef/planner/*`, `src/emuchef/steps/planner_hooks.py`, Python golden generation in crate README | `crates/emuchef-rust-backend/src/planner.rs`, `crates/emuchef-rust-backend/src/planner_tests.rs`, `crates/emuchef-rust-backend/tests/phase6m_planner.rs`, `crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6m_*.json`, `phase6n_*.json` | Ready with fixture-scoped coverage | Unit tests compare Python-shaped `PlanningResult`/`ExecutionPlan` for dependency expansion, namespacing, defaults, optional/multiple inputs, artifacts/groups, built-ins, and error/status shape. Protocol tests assert planner remains internal-only. | Internal-only; no protocol/API/CLI planning replacement; fixture-loaded authored roots only. | `Python deletion blocker`, `release confidence risk`, `accepted limitation` | Not a Tauri editor cutover blocker because current Tauri editor does not expose planner APIs; blocks Python CLI deletion. |
| Executor | `src/emuchef/executor/*`, `src/emuchef/steps/handlers/*`, Python executor goldens | `crates/emuchef-rust-backend/src/executor.rs`, `crates/emuchef-rust-backend/src/executor_tests.rs`, `crates/emuchef-rust-backend/src/executor_real_adb_tests.rs`, `crates/emuchef-rust-backend/tests/phase6o_protocol.rs` | Manual-gated | Normal tests cover dry-run, filesystem/artifact temp-root safety, fake-device/DryRunAdb, selected app/permission behavior, and protocol non-exposure. Seven real-ADB tests are ignored/manual. | Real-device/app/permission behavior is not fully proven; manual tests were not run in Phase 6T. | `release confidence risk`, `Python deletion blocker` | Not a current Tauri editor blocker; required before production executor confidence and Python CLI deletion. |
| CLI | `src/emuchef/cli.py`, `src/emuchef/io/execution_plan_io.py`, `pyproject.toml` | `crates/emuchef-rust-backend/src/cli.rs`, `crates/emuchef-rust-backend/tests/phase6s_cli.rs` | Partial | Tests and smokes cover Rust `validate` file mode, `apply --dry-run` selected plans, stdout/stderr/exit codes, and legacy JSON/sidecar dispatch preservation. | Rust lacks `draft`, `plan`, `detect`, `detect-profiles`, full catalog validation, real apply, ADB, verbose/debug, broad plan inputs/artifacts. | `Python deletion blocker`, `release confidence risk`, `accepted limitation` | Not a Tauri cutover blocker; blocks replacing/deleting Python CLI. |
| Tauri integration | `apps/config-editor/src-tauri/src/sidecar_client.rs`, `src-tauri/src/commands.rs`, `apps/config-editor/src/App.tsx` | `crates/emuchef-rust-backend` dev binary and Tauri v2 `externalBin` package artifact | 6U local/dev integrated; 6V host packaged sidecar integrated | Tauri Rust tests launch the actual Rust sidecar binary, verify the compatibility gate, cover stateless requests before/after sidecar startup, exercise open/get/apply/validate/emit/save/saveAs/close, assert the runtime command path does not spawn Python/uv, and run the same sequence through a simulated packaged sidecar directory. | GUI packaged E2E, signing/notarization, updater support, cross-platform package matrix, and public release hardening remain deferred. | `release confidence risk` | 6U resolves local/dev hard integration; 6V resolves host-target bundled sidecar launchability without claiming public release readiness. |
| Packaging / distribution | `apps/config-editor/src-tauri/tauri.conf.json`, `apps/config-editor/scripts/prepare-rust-sidecar.mjs`, Tauri v2 docs | `crates/emuchef-rust-backend/Cargo.toml` | Host-target sidecar bundling ready | `npm run tauri build` passed on macOS aarch64, produced `EmuChef Config Editor.app`, bundled `Contents/MacOS/emuchef-rust-backend`, and the bundled backend returned a successful `hello` response. The app-local script validates `rustc --print host-tuple`, builds debug/release sidecars, writes target-triple-suffixed `externalBin` inputs, and checks artifact freshness. | Cross-platform package verification, signing/notarization, installer/update behavior, release CI, and full GUI packaged E2E remain deferred. | `release confidence risk` | Host-target 6V packaging is no longer blocked for the Tauri editor sidecar; public release packaging remains incomplete. |
| Python / PySide6 removal | `src/emuchef`, `src/emuchef_editor`, `tests/test_editor_app.py`, `pyproject.toml` | Rust crate only partially replaces surfaces | Blocked | Python CLI entrypoints remain `emuchef` and `emuchef-editor`; PySide6 optional extra remains; Python goldens still regenerate Rust fixtures. | CLI, planner/executor breadth, PySide editor, templates, tests, docs, and golden generation still depend on Python. | `Python deletion blocker` | Deletion is a later confirmed-cutover phase, not 6T or 6U. |
| CI / tests | `tests/`, `apps/config-editor/tests`, Tauri Rust tests | Rust crate tests and Tauri Rust tests | Ready with fixture-scoped coverage | `cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml` passed; `cd apps/config-editor/src-tauri && cargo test` passed. | Manual real-ADB not run; Python golden regeneration not run; fixture breadth still limited. | `release confidence risk` | Good enough to proceed to 6U; not enough for public release alone. |

## Test Evidence

| Test suite / command | Required for normal verification | Requires Python | Requires device/ADB | Expected status | Last run status | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml` | Yes | No | No | Pass | Passed | 52 unit tests passed, 146 integration tests passed, 7 real-ADB tests ignored/manual. |
| One-shot hello smoke | Yes | No | No | Pass | Passed | `cargo run --quiet --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"hello"}'`; returned protocol v1 and exact Rust capability list. |
| One-shot `listStepSpecs` smoke | Yes | No | No | Pass | Passed | Returned nine Python fixture-backed StepSpecs. |
| One-shot `emitRecipeYamlFromPath` smoke | Yes | No | No | Pass | Passed | Used `crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml`; returned canonical YAML. |
| One-shot `validateRecipePath` smoke | Yes | No | No | Pass | Passed | Used `minimal_recipe.yaml`; returned expected limited-context warning without errors. |
| Rust CLI `validate` success smoke | Yes | No | No | Pass | Passed | `validate minimal_recipe.yaml` exited 0 with warning status and validated path output. |
| Rust CLI `validate` failure smoke | Yes | No | No | Pass | Passed | `validate invalid_top_level_permissions.yaml` exited 1 with top-level permissions diagnostic. |
| Rust CLI `apply --dry-run` success smoke | Yes | No | No | Pass | Passed | Temp execution plan created with `mktemp` plus `printf` fixture lines; command exited 0 with dry-run success summary. |
| Rust CLI `apply --dry-run` failure smoke | Yes | No | No | Pass | Passed | Same temp plan run without `--dry-run`; exited 1 with deferred real-apply error. |
| `cd apps/config-editor/src-tauri && cargo test` | Yes | No | No | Pass | Passed | 25 Tauri Rust tests passed, including sidecar compatibility, packaged/dev resolver, and bundled sidecar smoke tests. |
| `cd apps/config-editor && npm run sidecar:dev` | Yes | No | No | Pass | Passed | Built the debug Rust sidecar and prepared `src-tauri/binaries/emuchef-rust-backend-aarch64-apple-darwin` for Tauri v2 `externalBin`. |
| `cd apps/config-editor && npm run sidecar:build` | Yes for packaging | No | No | Pass | Passed | Built the release Rust sidecar, verified `rustc --print host-tuple`, wrote freshness metadata, and prepared the host-target externalBin artifact. |
| `cd apps/config-editor && npm run tauri build` | Yes for packaging | No | No | Pass | Passed | Built frontend and Tauri release app, produced `EmuChef Config Editor.app` and DMG on macOS aarch64, and included `Contents/MacOS/emuchef-rust-backend`. |
| Bundled sidecar `hello` smoke | Yes for packaging | No | No | Pass | Passed | Running `EmuChef Config Editor.app/Contents/MacOS/emuchef-rust-backend '{"type":"hello"}'` returned protocol v1 and the Rust capability list. |
| Simulated packaged sidecar editor smoke | Yes | No | No | Pass | Passed | Tauri Rust test copies the real Rust backend to a temp packaged sidecar directory, verifies platform name/Unix executable bit, and exercises `hello`, open/get/apply/validate/emit/save/saveAs/close through packaged resolution. |
| Manual real-ADB tests | No | No | Yes | Not run by default | Not run | Ignored tests require explicit env/device/test package configuration. |
| Python golden generation commands | No | Yes | No for most goldens | Not required in normal verification | Not run | Existing committed fixtures were used; StepSpec and editor-protocol freshness is a pre-6U merge expectation, while planner/executor/CLI goldens remain release-confidence or Python-deletion work unless 6U changes those surfaces. |

For the CLI dry-run smoke, the exact setup was a shell-created temp file:
`tmp_plan=$(mktemp)` followed by `printf` lines for a minimal `kind:
execution_plan` with one `wait` step, then `cargo run --quiet --manifest-path
crates/emuchef-rust-backend/Cargo.toml -- apply --plan-file "$tmp_plan"
--dry-run`.

## Manual-Test Evidence

| Manual test | Purpose | Required environment | Safe by default? | Last run | Result | Required before cutover? | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `phase6r_manual_real_adb_package_installed_check` | Verify package-installed query mapping against real ADB | `EMUCHEF_RUN_REAL_ADB_TESTS=1`, attached device/emulator, optional serial | Yes, read-only check | Not run | Not available | Required before production executor confidence, not normal CI | Ignored by default. |
| `phase6r_manual_real_adb_path_exists_check` | Verify real path-exists shell mapping | Same as above, optional `EMUCHEF_TEST_DEVICE_PATH` | Yes if using benign path | Not run | Not available | Required before production executor confidence | Ignored by default. |
| `phase6r_manual_real_adb_runtime_permission_requires_allowlist` | Verify permission command safety gates | Device, package allowlist, permission env vars | No, mutating unless explicitly allowed | Not run | Not available | Required before public real-device executor release | Must not run in normal CI. |
| `phase6r_manual_real_adb_appops_requires_allowlist` | Verify appops safety gates | Device, package allowlist, appop env vars | No, mutating unless explicitly allowed | Not run | Not available | Required before public real-device executor release | Must not run in normal CI. |
| `phase6r_manual_real_adb_install_apk_requires_explicit_opt_in` | Verify APK install path | Device, explicit APK and install opt-in | No, mutating | Not run | Not available | Required before production apply support | Not required for current Tauri editor cutover. |
| `phase6r_manual_real_adb_launch_app_requires_explicit_opt_in` | Verify app launch path | Device, explicit package/activity opt-in | No, mutating | Not run | Not available | Required before production apply support | Not required for current editor-only 6U. |
| `phase6r_manual_real_adb_force_stop_app_requires_explicit_opt_in` | Verify force-stop path | Device, explicit package opt-in | No, mutating | Not run | Not available | Required before production apply support | Not required for current editor-only 6U. |

## Known Gaps And Classification

| Gap | Classification | Blocker rank | Next action |
| --- | --- | --- | --- |
| Tauri local/dev runtime sidecar launch now uses Rust. | Resolved 6U local/dev blocker | N/A | Keep the hard-cutover sidecar process path; no selector or Python fallback is present. |
| Rust implements `saveRecipeAs` while Tauri registers `sidecar_save_recipe_as`. | Resolved 6U Tauri blocker | N/A | Keep `saveRecipeAs` in Tauri required capabilities because the command is registered; continue deferring `createRecipeFromTemplate` unless a Tauri caller is added. |
| Rust binary is bundled for the host-target Tauri editor build. | Resolved 6V host packaging blocker | N/A | Keep cross-platform package verification, signing/notarization, installer/update behavior, release CI, and GUI packaged E2E in 6X/release-hardening work. |
| Rust CLI is only a selected `validate` / `apply --dry-run` subset. | `Python deletion blocker` | 1 before Python deletion | Phase 6W or earlier CLI parity phase: replace `emuchef` CLI behavior or explicitly retire commands. |
| Planner and executor are internal fixture-scoped surfaces. | `Python deletion blocker`, `release confidence risk` | 2 before Python deletion | Broaden tests and production surfaces before removing Python planner/executor. |
| Real-ADB behavior is manual-gated and not run in normal tests. | `release confidence risk` | 1 before public apply release | Run manual real-device matrix before exposing production apply/device behavior. |
| Python-generated fixtures and goldens can drift. | `release confidence risk` | 2 before release confidence | Before merging 6U, regenerate StepSpec and editor-protocol goldens or record why committed fixtures remain authoritative; defer planner/executor/CLI golden refreshes to release-confidence or Python-deletion work unless 6U changes those surfaces. |
| `createRecipeFromTemplate` is Python-only. | `Python deletion blocker`, `accepted limitation for Tauri cutover` | 3 before Python deletion | Preserve, replace, or intentionally retire template creation before deleting Python/PySide6. Do not implement `createRecipeFromTemplate` in 6U unless Tauri integration discovers an active frontend/menu caller. |

## Ranked Cutover Blockers

### Before Tauri Hard Cutover

1. Phase 6U resolved the local/dev Tauri Rust sidecar launch path without adding
   a backend selector, backend toggle, environment variable, UI switch, protocol
   negotiation, or Python fallback.
2. Phase 6U resolved the registered-command gap by implementing Rust
   `saveRecipeAs` and adding it to Tauri's required capabilities.
3. Phase 6U added an automated Tauri-side smoke that launches the real Rust
   sidecar binary and exercises required editor flows.
4. Host-target packaged sidecar launchability is resolved by 6V. Full GUI
   packaged-app E2E remains release-confidence work.

6U must verify the actual Tauri-launched Rust sidecar binary, not only
`cargo run` or crate-local tests. The smoke should cover:

- `hello` compatibility check.
- `openRecipe`.
- `getDocument`.
- `applyRecipeCommand` with a no-op or overview edit.
- `validate`.
- `emitYaml`.
- `saveRecipe` on a temp copy.
- `saveRecipeAs` to a temp target.
- `closeDocument`.

### Before Production Packaging

1. Host-target Rust sidecar binary build and Tauri bundling are resolved by 6V.
2. Cross-platform package verification remains open for Windows, Linux, and
   alternate macOS architectures.
3. Signing/notarization implications for macOS and platform installers remain
   open.
4. Install/update flow and rollback package strategy remain open.
5. Release CI that builds and verifies the binary on target platforms remains
   open.

### Before Python/PySide6 Deletion

1. Replace or retire Python `emuchef` CLI commands: `draft`, `plan`, `detect`,
   `detect-profiles`, broad `validate`, and real `apply`.
2. Replace Python planner/executor behavior or document its retirement.
3. Replace Python golden generation strategy or keep a generator-only Python
   tool with explicit ownership.
4. Replace or retire PySide6 editor features, including Save As and
   create-from-template behavior.
5. Remove/update Python-only docs, scripts, tests, and package metadata only
   after verified Rust cutover.

### Before Public / Release Confidence

1. Run manual real-device executor coverage for selected ADB/device/app flows.
2. Expand authoredRoot project coverage beyond fixture slices.
3. Refresh all Python-generated fixtures/goldens.
4. Verify Windows/macOS/Linux filesystem and path behavior.
5. Run end-to-end Tauri editor tests against packaged Rust sidecar builds.

## Fixture And Golden Freshness

| Fixture/golden | Location | Regeneration | Runtime Python required by normal Rust tests? | Freshness risk |
| --- | --- | --- | --- | --- |
| StepSpec fixture | `tests/fixtures/python_step_specs.json` | README command using `PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef_editor.api.server '{"type":"listStepSpecs"}'` | No | High if Python StepSpecs change. |
| YAML emit/validate goldens | `tests/fixtures/python_goldens/*.emit.yaml`, `*.validate.json` | README Python snippets invoking Python API/server behavior | No | Medium; serde_yaml/PyYAML differences can hide outside fixtures. |
| Session/command/RefIndex goldens | `phase6g_*`, `phase6h_*`, `phase6i_*` files | README Python scripts using `PYTHONPATH=src:tests` | No | Medium; refresh after command or DTO changes. |
| Planner goldens | `phase6m_*`, `phase6n_*` JSON files | README planner golden generation commands | No | High before Python deletion because planner remains internal/fixture-scoped. |
| Executor goldens | `phase6o_*`, `phase6p_*`, `phase6q_*` JSON files | README executor golden generation commands | No | High before production apply/device behavior. |
| CLI checked expectations | `tests/phase6s_cli.rs` | Python CLI commands documented in crate README | No | Medium; refresh after CLI text changes. |

No fixture or golden regeneration was run in Phase 6T. Normal Rust tests used
the committed fixtures and did not require Python at runtime. Before merging 6U,
regenerate StepSpec and editor-protocol goldens, or explicitly record why the
committed fixtures remain authoritative for 6U. Planner, executor, and CLI golden
refreshes remain release-confidence and Python-deletion work unless 6U changes
those surfaces.

## Binary / Entrypoint Inventory

| Entrypoint | Path/source | Status | Notes |
| --- | --- | --- | --- |
| Python CLI `emuchef` | `pyproject.toml` -> `src/emuchef/cli.py` | Production/current | Owns full CLI surface. |
| Python PySide editor `emuchef-editor` | `pyproject.toml` -> `src/emuchef_editor/app/app.py` | Legacy/current | Depends on PySide6 optional extra. |
| Python one-shot editor API | `python -m emuchef_editor.api.server '<json>'` | Legacy/reference | No longer used by Tauri editor runtime after 6U; Python remains for CLI/PySide/golden work. |
| Python JSONL sidecar | `python -m emuchef_editor.api.server --sidecar` | Legacy/reference | No Tauri runtime fallback after 6U; deletion remains a later 6W concern. |
| Rust crate binary | `crates/emuchef-rust-backend/src/main.rs` | Experimental | Cargo package binary name is `emuchef-rust-backend`. |
| Rust one-shot JSON mode | `emuchef-rust-backend '<json>'` | Experimental/test | Smoke-tested in Phase 6T. |
| Rust JSONL sidecar mode | `emuchef-rust-backend --sidecar` | Experimental local/dev Tauri runtime | Covered by crate tests and Tauri-launched sidecar smoke; production bundling remains 6V. |
| Rust CLI `validate` | `emuchef-rust-backend validate <recipe>` | Experimental crate-local | Selected explicit-file parity only. |
| Rust CLI `apply --dry-run` | `emuchef-rust-backend apply --plan-file <path> --dry-run` | Experimental crate-local | Selected plan fixtures only; not production apply. |
| Tauri command bridge | `apps/config-editor/src-tauri/src/commands.rs` | Local/dev Rust sidecar runtime | Former one-shot and persistent editor commands route through the Rust sidecar state. |
| Fixture/golden generation commands | crate README Python snippets | Test/developer-only | Python remains required to regenerate goldens. |

## Rollback

Because there is no backend selector or toggle, rollback after hard cutover means
reverting the cutover commit and shipping a rollback package. Rollback is not a
runtime option. The cutover phase should minimize unrelated changes so revert is
simple. Packaging and release plans must account for binary rollback, including
removing or replacing the Rust sidecar binary from packaged assets.

## Blockers By Destination

| Destination | Blocking items |
| --- | --- |
| Tauri hard cutover | Phase 6U resolves local/dev editor runtime blockers: Rust sidecar launch path, `saveRecipeAs` registered-command gap, Tauri-launched Rust binary smoke, compatibility verification against the real Rust binary, and command/API regression coverage. Phase 6V adds host-target bundled sidecar launchability. GUI packaged E2E remains release-confidence work. |
| Production packaging | Host-target Tauri sidecar bundling is resolved for the editor path. Cross-platform binary/package verification, signing/notarization, installer/update path, release CI, and package rollback plan remain. |
| Python/PySide6 deletion | Full CLI replacement/retirement, planner/executor breadth, template creation and Save As disposition, Python golden-generation strategy, Python tests/docs/scripts cleanup. |
| Public release confidence | Manual real-device matrix, broader authoredRoot coverage, fixture/golden refresh, cross-platform filesystem/path validation, packaged-app E2E tests. |

## Recommended Next Phases

| Phase | Goal | High-level scope | Explicit non-goals | Resolves |
| --- | --- | --- | --- | --- |
| 6U | Hard-integrate Tauri with Rust sidecar, no selector | Replace Python sidecar launch with Rust, resolve `saveRecipeAs`, verify required capabilities, and run a Tauri-launched Rust binary smoke covering `hello`, `openRecipe`, `getDocument`, `applyRecipeCommand`, `validate`, `emitYaml`, `saveRecipe`, `saveRecipeAs`, and `closeDocument` | No packaging overhaul, no Python deletion, no backend toggle | Tauri local/dev hard-cutover blockers. |
| 6V | Package and bundle Rust backend | Build host-target sidecar binary, bundle in Tauri via v2 `externalBin`, document dev/build packaging commands and evidence | No Python deletion, no backend selector, no broad release hardening | Host-target Tauri sidecar packaging blockers. |
| 6W | Remove or retire Python backend/PySide6 after verified Rust cutover | Replace/retire Python CLI, PySide editor, Python sidecar, tests/docs/scripts, golden strategy | No runtime backend selector | Python deletion blockers. |
| 6X | Cleanup, CI, and release hardening | Broaden CI, refresh goldens, cross-platform package verification, signing/notarization/update docs, GUI packaged E2E, release documentation | No new broad product features | Release confidence risks. |

## Risk Register

| Risk | Impact | Likelihood | Mitigation | Owner/follow-up |
| --- | --- | --- | --- | --- |
| Fixture coverage is not broad enough. | Rust may pass tests but fail real authored projects. | High | Add broader authoredRoot and editor workflow fixtures before release. | 6U/6X |
| YAML/message parity is semantic, not always byte-for-byte. | Diff churn or unexpected diagnostics may appear. | Medium | Use structured comparisons where intended; document exact text requirements. | 6U/6X |
| Manual real-device coverage is incomplete. | Production apply/device behavior can regress. | High | Keep manual tests out of normal CI but require a pre-release manual matrix. | 6X |
| Tauri sidecar launch assumptions differ from crate-local process tests. | Rust binary can work in tests but fail in app launch context. | Medium | 6U must test the actual Tauri-launched binary. | 6U |
| Packaging/codesigning complexity delays release. | Rust cutover may work locally but still not meet public release requirements. | Medium | 6V proves host-target bundled sidecar viability; keep signing/notarization, package rollback, updater, and cross-platform CI in 6X. | 6X |
| Python golden generation still depends on Python. | Python deletion can remove test regeneration path. | High | Decide whether to preserve a generator tool, replace goldens, or freeze fixtures. | 6W |
| Hidden Python-only scripts or docs remain. | Deleting Python breaks developer workflows. | Medium | Search scripts/docs before 6W; update or remove references. | 6W |
| No selector means rollback requires revert/package rollback. | Failed release cannot be fixed by flipping config. | High | Keep cutover commits small and release rollback package ready. | 6U/6V |
| Cross-platform filesystem/path behavior differs. | Windows/macOS/Linux bugs in YAML, executor, or CLI paths. | Medium | Add platform CI and path fixtures. | 6X |
| Optional capability assumptions are missed. | Registered frontend/Tauri command fails after cutover. | Medium | 6U treats `saveRecipeAs` as required because the Tauri command is registered; apply the same rule to future registered commands. | 6U+ |

## Audit And Packaging Fixes Made

Phase 6T made no Rust behavior changes. Phase 6V changes are limited to Tauri
sidecar packaging, app-local build orchestration, sidecar path resolution,
packaging smoke coverage, and documentation updates. They do not add a backend
selector, Python fallback, Python deletion, protocol negotiation, direct-library
integration, or broad planner/executor/CLI behavior.

## Final Verdict

**6U and host-target 6V sidecar packaging are complete for the Tauri editor
path.**

This verdict is evidence-led: the Rust crate tests passed, one-shot and CLI
smokes passed, Tauri Rust tests passed, `npm run tauri build` produced a macOS
aarch64 app bundle with `emuchef-rust-backend`, and the bundled backend returned
a valid `hello` response. The verdict does not authorize Python deletion,
signing/notarization claims, updater support, cross-platform release automation,
or public release readiness.

Top 5 blockers:

1. Tauri local/dev runtime no longer launches the Python sidecar. Phase 6U
   hard-integrates the Rust sidecar with no selector or fallback. Phase 6V adds
   host-target Tauri bundled sidecar launchability.
2. Rust now implements `saveRecipeAs`, and Tauri requires the capability because
   `sidecar_save_recipe_as` is registered. This resolves the 6T registered
   command cutover blocker.
3. Cross-platform packaging, signing/notarization, installer/update behavior,
   release CI, and GUI packaged E2E are still incomplete. Next action: 6X
   release hardening.
4. Python CLI/planner/executor breadth is not replaced. Next action: 6W or a
   dedicated CLI/planner/executor parity phase. Prevents Python deletion.
5. Manual real-device executor evidence is absent. Next action: run the
   Phase 6R manual matrix before exposing production apply/device behavior.
   Affects release confidence.
