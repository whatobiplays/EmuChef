# Rust Launcher-Injected Planner Readiness Intake Design

## Purpose

P8BB is documentation-only. It defines the future readiness-gate intake rules
for `rust_launcher_injected_planner_smoke` reports produced by the P8BA smoke
tool.

Readiness intake is not implemented. No report kind is accepted yet, no
readiness blocker is cleared, and packaged release readiness remains future
work.

P8BA report identity remains:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
```

## Future Acceptance Rules

A future readiness gate should accept a report only when all of these
conditions are true:

1. `kind == rust_launcher_injected_planner_smoke`
2. `schema_version == 1`
3. `summary.failed == 0`
4. `inputs.planner_backend == rust-production-equivalent`
5. `inputs.launcher_supplied_planner_path == true`
6. `inputs.path_was_absolute == true`
7. `inputs.path_exists == true`
8. `inputs.path_executable == true`
9. `inputs.argv0_corresponds_to_launcher_path == true`
10. `inputs.explicit_python_bypass_checked == true`
11. `inputs.explicit_python_bypass_check_mode == cli_help_static`
12. all required checks are present and passed
13. all redaction booleans are true
14. `artifacts.argv0_basename` is present and contains no `/` or `\`

The future gate should continue to require the existing top-level report shape:
`kind`, `schema_version`, `generated_at`, `summary`, `inputs`, `checks`,
`redaction`, and `artifacts`.

Additional non-sensitive fields may be ignored by the future readiness gate
unless a later implementation explicitly makes them acceptance criteria.

## Required Checks

The future gate should require these checks by exact name, and each check must
have `passed: true`:

1. `launcher_supplied_path_absolute`
2. `launcher_supplied_path_exists`
3. `launcher_supplied_path_file`
4. `launcher_supplied_path_executable`
5. `argv0_corresponds_to_launcher_path`
6. `known_fixture_plan_succeeded`
7. `explicit_python_backend_bypass_available`
8. `no_implicit_fallback_sources_used`

## Future Rejection Rules

The future gate should reject reports with any of these conditions:

1. denylisted sensitive keys or equivalent fields
2. full local path-looking string values
3. `artifacts.argv0_basename` containing `/` or `\`
4. missing required top-level keys
5. unknown `schema_version`
6. `summary.failed > 0`
7. missing required checks
8. failed required checks

Denylisted sensitive keys are:

1. `command`
2. `argv`
3. `raw_command`
4. `stdout`
5. `stderr`
6. `raw_stdout`
7. `raw_stderr`
8. `environment`
9. `env`
10. `serial`
11. `device_serial`
12. `planner_path`
13. `absolute_path`
14. `launcher_supplied_absolute_path`
15. `cwd`
16. `home`

Equivalent fields are alternate names that carry the same sensitive values.
Reports must not store raw commands, process output, environment data, device
identifiers, home directories, working directories, or planner paths under
different field names.

Full local path-looking string values must be rejected even when the field name
is not denylisted. Safe basenames such as `emuchef-plan-shadow` remain allowed
when they do not reveal a local path.

## Future Blocker Semantics

An accepted report may satisfy the packaged launcher-injection evidence bar
later, but it does not clear executor/apply readiness, Python planner deletion
readiness, broader packaged release readiness, or any blocker in P8BB.

P8BB itself clears no blocker. A future implementation must explicitly wire
readiness intake before `rust_launcher_injected_planner_smoke` reports can
change readiness output.
