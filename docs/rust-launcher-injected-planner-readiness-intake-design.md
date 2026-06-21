# Rust Launcher-Injected Planner Readiness Intake Design

## Purpose

P8BB was documentation-only and defined the readiness-gate intake rules for
`rust_launcher_injected_planner_smoke` reports produced by the P8BA smoke tool.

P8BC implements supplied-report readiness intake for
`rust_launcher_injected_planner_smoke` reports with `schema_version: 1`. The
readiness gate accepts this report kind only when a developer explicitly
supplies a report path. P8BC does not execute the P8BA smoke tool, import smoke
tool code, execute planner code, probe devices, read `.local` evidence
implicitly, change CLI resolver behavior, implement packaged lookup, change
runtime behavior, or change packaging behavior.

P8BA report identity remains:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
```

## Acceptance Rules

The readiness gate accepts a report only when all of these conditions are true:

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

The gate requires the existing top-level report shape:
`kind`, `schema_version`, `generated_at`, `summary`, `inputs`, `checks`,
`redaction`, and `artifacts`.

Additional non-sensitive fields may be ignored by the readiness gate unless a
later implementation explicitly makes them acceptance criteria.

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

## Rejection Rules

The gate rejects reports with any of these conditions:

1. denylisted sensitive keys
2. full local path-looking string values
3. `artifacts.argv0_basename` containing `/` or `\`
4. missing required top-level keys
5. unknown `schema_version`
6. `summary.failed > 0`
7. missing required checks
8. failed required checks

Denylisted sensitive keys are matched by exact normalized key only, equivalent
to `key.lower() in DENYLISTED_KEYS`. Allowed fields such as `argv0_basename`
and `argv0_corresponds_to_launcher_path` are not rejected merely because they
contain `argv0`.

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

Full local path-looking string values are rejected even when the field name is
not denylisted. The heuristic is intentionally bounded to obvious local paths:
absolute POSIX paths beginning with `/`, home-relative paths beginning with
`~/`, Windows drive paths such as `C:\...` or `C:/...`, and UNC paths beginning
with `\\`. Safe schema values such as `rust-production-equivalent`,
`cli_help_static`, `temporary_fixture_json`, `external_wrapper`,
`emuchef-plan-shadow`, and ISO-8601 timestamps remain allowed when they do not
reveal local host paths.

For `artifacts.argv0_basename`, the gate applies the stricter basename rule:
the value must be a non-empty string and must not contain `/` or `\`.

## Readiness Semantics

An accepted P8BC report may satisfy only the packaged launcher-injection
evidence item. It does not clear executor/apply readiness, Python planner
deletion readiness, broader packaged release readiness, or top-level readiness.
Top-level readiness remains `blocked` while those blockers remain.

`docs/rust-packaged-readiness-blocker-taxonomy.md` defines the P8BD taxonomy for
separating accepted launcher-injection evidence from broader packaged release
readiness, executor/apply readiness, Python planner deletion readiness, and
top-level readiness.
