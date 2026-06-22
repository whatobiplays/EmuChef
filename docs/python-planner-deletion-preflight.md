# Python Planner Deletion Preflight

P8BI adds `tools/check_python_planner_deletion_preflight.py` as a static,
developer-only preflight for the Python planner deletion sequence.

P8BI is static only. It does not delete Python planner code, change runtime
behavior, import `emuchef`, execute planner code, invoke subprocesses, probe
devices, read `.local`, or require Rust binaries. It identifies remaining
Python planner deletion surfaces in checked-in source, tests, readiness docs,
and current-state project context.

The preflight emits deterministic JSON with
`kind: python_planner_deletion_preflight`, `schema_version: 1`, and a top-level
`status`. The status is `blocked` while any remaining surface is present and
`ready` only when no remaining surfaces remain. A blocked report lists the
remaining Python planner surfaces and the deletion steps that future slices must
complete.

The preflight exists to guide and verify the Python planner deletion sequence.
It makes the remaining CLI runtime path, CLI imports, readiness blocker,
fallback assertions, and docs/context references explicit before any deletion
slice removes or quarantines them. Once Python planner deletion is complete, the
preflight should either report ready with no remaining surfaces or be removed as
part of cleanup.

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
