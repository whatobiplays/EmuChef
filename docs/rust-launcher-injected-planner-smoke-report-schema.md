# Rust Launcher-Injected Planner Smoke Report Schema

## Purpose

P8AZ is documentation-only. It defines the report schema for launcher-injected
planner smoke evidence.

P8BA implements the smoke tool and tests for this schema only. P8BA does not
add readiness intake, clear a readiness blocker, change CLI resolver behavior,
implement packaged lookup, change packaging scripts, change Tauri
configuration, change Rust backend code, change executor/apply behavior, or
write `.local` evidence.

The report identity is:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
```

No report of this kind is accepted readiness evidence until a later readiness
gate explicitly accepts it.

P8BB is documentation-only and records
`docs/rust-launcher-injected-planner-readiness-intake-design.md` as the future
readiness intake design for this report kind. Readiness intake is not
implemented, no report kind is accepted yet, no readiness blocker is cleared,
and packaged release readiness remains future work.

## Required Top-Level Keys

`rust_launcher_injected_planner_smoke` reports must include these top-level
keys:

1. `kind`
2. `schema_version`
3. `generated_at`
4. `summary`
5. `inputs`
6. `checks`
7. `redaction`
8. `artifacts`

## Required Report Shape

Reports must use this shape:

```yaml
kind: rust_launcher_injected_planner_smoke
schema_version: 1
generated_at: "<ISO-8601 timestamp>"
summary:
  passed: 8
  failed: 0
inputs:
  planner_backend: rust-production-equivalent
  launcher_supplied_planner_path: true
  path_was_absolute: true
  path_exists: true
  path_is_file: true
  path_executable: true
  argv0_corresponds_to_launcher_path: true
  explicit_python_bypass_checked: true
  explicit_python_bypass_check_mode: cli_help_static
  detected_facts_source: temporary_fixture_json
  launcher_entrypoint_observation: external_wrapper
checks:
  - name: launcher_supplied_path_absolute
    passed: true
  - name: launcher_supplied_path_exists
    passed: true
  - name: launcher_supplied_path_file
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
  process_invocation_omitted: true
  process_output_omitted: true
  runtime_context_omitted: true
  device_identifiers_omitted: true
  sensitive_values_omitted: true
artifacts:
  argv0_basename: emuchef-plan-shadow
```

`summary.passed` and `summary.failed` are integer counts populated by the smoke
tool. Failure reports keep the same top-level shape and return nonzero when any
required check fails.

## Field Semantics

`launcher_supplied_planner_path: true` means the launcher supplied the planner
binary path through the existing `--rust-planner-bin <path>` option.

`argv0_corresponds_to_launcher_path` means the CLI invoked the
launcher-supplied executable path as the Rust planner subprocess entrypoint. In
P8BA that executable may be an observation wrapper around the real planner
binary, so this field does not claim the wrapped real planner binary path was
proved directly.

`argv0_basename` may be reported because it is not a local path.

`explicit_python_backend_bypass_available` is static in P8BA. The smoke checks
that CLI help still exposes explicit `--planner-backend python` as a
bypass/reference route and does not execute that backend.

`no_implicit_fallback_sources_used` means the smoke invoked the explicit Rust
route with an explicit launcher-supplied planner path and observed that path
being used. The smoke does not rely on `PATH`, env-var, Cargo, cwd, repo-local
lookup, or packaged helper lookup, and it does not claim internal resolver
instrumentation.

## Sensitive Field Denylist

Reports must not include these fields or equivalent alternative names
containing the same sensitive values, including failure reports:

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

The schema defines the smoke report shape. P8BA implements the smoke tool only;
it does not add readiness-gate intake or clear a readiness blocker.

P8BB documents future readiness intake rules only. The report identity remains
`rust_launcher_injected_planner_smoke` with `schema_version: 1`, and no report
kind is accepted yet.

Packaged release readiness remains future work until a later readiness gate
accepts this report kind as evidence.
