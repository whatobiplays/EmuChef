# Rust Launcher-Injected Planner Smoke Contract

## Purpose

P8AY is documentation-only. It defined the smoke evidence contract for
launcher-injected Rust planner binary paths through the existing
`--rust-planner-bin <path>` option.

P8AY does not implement a smoke tool, packaged lookup, runtime code, tests,
Tauri configuration, packaging scripts, readiness logic, Rust backend code, or
`.local` evidence.

P8AZ is documentation-only. It defines
`docs/rust-launcher-injected-planner-smoke-report-schema.md` as the schema for
`rust_launcher_injected_planner_smoke` reports with `schema_version: 1`.

P8BA implements `tools/smoke_launcher_injected_planner.py` and focused tests for
that smoke tool only. P8BA does not add readiness intake, clear a readiness
blocker, change CLI resolver behavior, implement packaged lookup, change Tauri
configuration, change packaging scripts, change Rust backend code, change
executor/apply behavior, or write `.local` evidence. Packaged release readiness
remains future work.

P8BB is documentation-only. It records
`docs/rust-launcher-injected-planner-readiness-intake-design.md` as the future
readiness intake design for `rust_launcher_injected_planner_smoke` reports with
`schema_version: 1`; readiness intake is not implemented, no report kind is
accepted yet, and no readiness blocker is cleared.

## Current State

P8AW selected a package/runtime-provided absolute planner binary path as the
future packaged Rust planner mechanism.

P8AX accepted launcher injection through the existing
`--rust-planner-bin <path>` option as the first packaged planner integration
path.

The current CLI resolver still uses only explicit-path validation.
`_packaged_rust_planner_bin_candidate(args)` still returns `None`. No packaged
lookup is implemented. `--rust-planner-bin` remains required unless a launcher
supplies it. Explicit `--planner-backend python` remains available.

The P8BA smoke tool exists for manual/developer evidence generation. It invokes
the explicit `rust-production-equivalent` planner route with an explicit
launcher-supplied `--rust-planner-bin <path>` value and a temporary
detected-facts fixture. The current CLI resolver still uses only explicit-path
validation; no packaged lookup is implemented.

P8BA does not add readiness intake and does not clear a readiness blocker.
Packaged release readiness remains future work.

P8BB does not implement readiness intake. The P8BA report identity remains
`rust_launcher_injected_planner_smoke` with `schema_version: 1`, no report kind
is accepted yet, and packaged release readiness remains future work.

## Smoke Goal

The smoke proves:

```text
a launcher/package layer can invoke emuchef plan with a launcher-supplied absolute planner binary path through --rust-planner-bin <path>
```

The launcher-supplied executable may be an argv0-observing wrapper around the
real planner binary. `argv0_corresponds_to_launcher_path` means the CLI invoked
the launcher-supplied executable path as the Rust planner subprocess entrypoint.
P8BA does not claim to prove the wrapped real planner binary path directly when
a wrapper is used.

## Required Smoke Assertions

The P8BA smoke asserts:

1. the launcher supplies an absolute path;
2. the path resolves to an existing file;
3. the path is executable;
4. the observed Rust planner subprocess argv[0], after redaction, corresponds
   to the launcher-supplied executable path;
5. the plan command succeeds for a known fixture/device-plan route;
6. explicit `--planner-backend python` remains exposed by CLI help as a static
   bypass/reference route without executing that backend;
7. no implicit fallback sources are used.

`no_implicit_fallback_sources_used` means the smoke invoked the explicit Rust
route with an explicit launcher-supplied planner path and observed that path
being used. The smoke does not rely on `PATH`, env-var, Cargo,
current-working-directory, repo-local lookup, or packaged helper lookup, and it
does not claim internal resolver instrumentation.

## Redaction Rules

The smoke emits a report even when path validation or planner execution fails,
and it returns nonzero when any required check fails. The smoke report must not
record:

1. full local absolute user paths;
2. device serials;
3. environment variables;
4. full command lines with local filesystem paths;
5. stdout or stderr containing local filesystem paths unless scrubbed;
6. raw process environment.

The smoke report may record normalized evidence such as:

```yaml
path_was_absolute: true
path_exists: true
path_executable: true
argv0_basename: emuchef-plan-shadow
argv0_corresponds_to_launcher_path: true
planner_backend: rust-production-equivalent
```

## Non-Goals

P8AY does not:

1. add a smoke script;
2. change readiness gate behavior;
3. clear packaged readiness blockers;
4. define Tauri packaging configuration;
5. define installer behavior;
6. change CLI resolver behavior;
7. implement packaged lookup;
8. touch `.local` evidence;
9. change tests, smoke tools, runtime code, or Rust backend code.

P8BA does not:

1. add readiness-gate intake for `rust_launcher_injected_planner_smoke`;
2. clear packaged readiness blockers;
3. change CLI resolver behavior;
4. implement packaged lookup;
5. change Tauri configuration or packaging scripts;
6. change Rust backend code;
7. change executor/apply behavior;
8. write `.local` evidence.

## Readiness Relationship

The P8BA smoke may become readiness evidence only after a later phase makes the
readiness gate accept its report kind.

P8BB documents the future intake criteria for that later phase but does not
implement them. P8BA and P8BB clear no blocker. Packaged release readiness
remains future work.
