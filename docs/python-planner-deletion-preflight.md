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
