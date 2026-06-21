# Rust Launcher-Injected Planner Smoke Report Schema

## Purpose

P8AZ is documentation-only. It defines the future report schema for
launcher-injected planner smoke evidence without implementing the smoke tool,
runtime code, tests, readiness intake, packaging scripts, Tauri configuration,
Rust backend changes, or `.local` evidence.

The future report identity is:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
```

No report of this kind is accepted evidence until a later smoke tool produces it
and a later readiness gate explicitly accepts it.

## Required Top-Level Keys

Future `rust_launcher_injected_planner_smoke` reports must include these
top-level keys:

1. `kind`
2. `schema_version`
3. `generated_at`
4. `summary`
5. `inputs`
6. `checks`
7. `redaction`
8. `artifacts`

## Required Report Shape

Future reports must use this shape:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
generated_at: "<ISO-8601 timestamp>"
summary:
  passed: 7
  failed: 0
inputs:
  planner_backend: rust-production-equivalent
  launcher_supplied_planner_path: true
  launcher_supplied_path_was_absolute: true
  explicit_python_bypass_checked: true
checks:
  - name: launcher_supplied_path_absolute
    passed: true
  - name: launcher_supplied_path_exists
    passed: true
  - name: launcher_supplied_path_executable
    passed: true
  - name: argv0_corresponds_to_launcher_path
    passed: true
  - name: known_fixture_plan_succeeded
    passed: true
  - name: explicit_python_backend_bypass_available
    passed: true
  - name: no_implicit_fallback_sources_used
    passed: true
redaction:
  full_paths_omitted: true
  serials_omitted: true
  environment_omitted: true
  raw_command_omitted: true
  stdout_stderr_scrubbed: true
artifacts:
  argv0_basename: emuchef-plan-shadow
```

`summary.passed` and `summary.failed` are integer counts populated by the future
smoke tool. The numeric values in this example are placeholders and do not imply
that future reports always have zero failures or a fixed number of passing
checks.

## Field Semantics

`launcher_supplied_planner_path: true` means the launcher supplied the planner
binary path through the existing `--rust-planner-bin <path>` option.

`argv0_corresponds_to_launcher_path` must be proved internally by the future
smoke tool and reported only as a boolean.

`argv0_basename` may be reported because it is not a local path.

`no_implicit_fallback_sources_used` means no packaged helper lookup, `PATH`,
env-var, Cargo, cwd, or repo-local fallback was used.

## Sensitive Field Denylist

Future reports must not include these fields or equivalent alternative names
containing the same sensitive values:

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

Full local path values must not be stored under alternative field names.

The report may record redacted boolean facts, stable classifications, and safe
basenames when those values do not reveal local host paths, raw commands, raw
stdout/stderr, environment data, or device serials.

## Readiness Relationship

The schema only defines the future report shape. It does not implement a smoke
tool, make any report available, add readiness-gate intake, or clear a readiness
blocker.

Packaged release readiness remains future work until a later implementation
produces this report kind and a later readiness gate accepts it as evidence.
