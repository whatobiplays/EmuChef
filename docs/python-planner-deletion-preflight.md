# Python Planner Deletion Preflight

P8BI adds `tools/check_python_planner_deletion_preflight.py` as a static,
developer-only preflight for the Python planner deletion sequence.

P8BI is static only. It does not delete Python planner code, change runtime
behavior, import `emuchef`, execute planner code, invoke subprocesses, probe
devices, read `.local`, or require Rust binaries. The current P8BO surface set
tracks public `emuchef plan` Python planner fallback, import, runtime,
readiness, and test surfaces in checked-in source.

The preflight emits deterministic JSON with
`kind: python_planner_deletion_preflight`, `schema_version: 1`, and a top-level
`status`. The status is `blocked` while any tracked public `emuchef plan`
Python planner fallback/import/runtime/readiness/test surface is present and
`ready` only when no tracked surfaces remain. A blocked report lists the
remaining Python planner surfaces and the deletion steps that future slices must
complete.

The preflight exists to guide and verify the Python planner deletion sequence.
It makes the remaining CLI runtime path, CLI imports, readiness blocker,
and fallback assertions explicit before any deletion slice removes or
quarantines them. `ready` does not mean `src/emuchef/planner.py` has been
deleted and does not mean Python draft/session/profile helper behavior has been
removed.

P8BJ removes the explicit public `--planner-backend python` route from
`emuchef plan`. It does not delete Python planner code, remove
`_run_python_plan`, remove CLI imports from `emuchef.planner`, or change draft
behavior, executor/apply behavior, Rust planner behavior, smoke tooling,
readiness validators, packaging, Tauri/config-editor behavior, or `.local`
evidence. After P8BJ, the real repository preflight no longer reports
`cli_explicit_python_backend` or `test_cli_explicit_python_backend_behavior`,
but it still reports `blocked` while other Python planner deletion surfaces
remain.

P8BK removes `_run_plan(...)` fallback routing to `_run_python_plan(...)`.
It does not delete `_run_python_plan(...)`, delete Python planner code, remove
CLI imports from `emuchef.planner`, or change draft behavior, executor/apply,
Rust planner behavior, smoke tooling, readiness validators, packaging,
Tauri/config-editor, Rust backend code, or `.local` evidence. After P8BK, the
real repository preflight no longer reports
`cli_run_plan_routes_to_python_plan`, but it still reports `blocked` while
other Python planner deletion surfaces remain.

P8BL deletes the private `_run_python_plan(...)` function and removes the
transitional direct tests of that private function. It does not delete
`src/emuchef/planner.py`, remove broader CLI imports from `emuchef.planner`, or
change draft behavior, executor/apply, Rust planner behavior, smoke tooling,
readiness validators, packaging, Tauri/config-editor, Rust backend code, or
`.local` evidence. After P8BL, the real repository preflight no longer reports
`cli_run_python_plan_function`, but it still reports `blocked` while other
Python planner deletion surfaces remain.

P8BM removes direct `src/emuchef/cli.py` imports from `emuchef.planner` by
adding `src/emuchef/planner_cli.py` as a transitional CLI-facing compatibility
module for still-needed Python planner symbols. It does not delete
`src/emuchef/planner.py`, remove Python draft/session/profile helper behavior,
or change plan routing, draft behavior, executor/apply, Rust planner behavior,
smoke tooling, readiness validators, packaging, Tauri/config-editor, Rust
backend code, or `.local` evidence. After P8BM, the real repository preflight
no longer reports `cli_imports_emuchef_planner`, but it still reports
`blocked` while non-runtime readiness/test/docs/context surfaces remain.

P8BN removes stale Rust readiness and launcher-smoke assumptions that explicit
`--planner-backend python` remains available. It removes
`python_planner_deletion_not_ready` as a Rust planner cutover readiness blocker
and removes the real-repo readiness-test and launcher-smoke preflight surfaces.
It does not delete `src/emuchef/planner.py`, remove Python
draft/session/profile helper behavior, or change plan routing, draft behavior,
executor/apply, Rust planner behavior, packaging, Tauri/config-editor, Rust
backend code, or `.local` evidence. After P8BN, the real repository preflight
still reports `blocked` for the remaining docs/context surfaces until those
surfaces are intentionally retired.

P8BO retires the final docs/context preflight detector surfaces. After P8BO,
the real repository preflight reports `ready` with no remaining surfaces and no
required deletion steps. This means the Python planner deletion preflight is
complete for tracked public `emuchef plan`
fallback/import/runtime/readiness/test surfaces after P8BO. P8BO does not
delete `src/emuchef/planner.py`, remove Python draft/session/profile helper
behavior, or change plan routing, draft behavior, executor/apply, Rust planner
behavior, smoke tooling, readiness validators, packaging, Tauri/config-editor,
Rust backend code, or `.local` evidence.
