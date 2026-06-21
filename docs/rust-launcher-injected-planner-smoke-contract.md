# Rust Launcher-Injected Planner Smoke Contract

## Purpose

P8AY is documentation-only. It defines future smoke evidence for
launcher-injected Rust planner binary paths through the existing
`--rust-planner-bin <path>` option.

P8AY does not implement a smoke tool, packaged lookup, runtime code, tests,
Tauri configuration, packaging scripts, readiness logic, Rust backend code, or
`.local` evidence.

P8AZ is documentation-only. It defines
`docs/rust-launcher-injected-planner-smoke-report-schema.md` as the future
schema for `rust_launcher_injected_planner_smoke` reports with
`schema_version: 1`. No smoke tool is implemented, no readiness intake is
implemented, no readiness blocker is cleared, and no full local paths, serials,
commands, stdout/stderr, or environment variables may be recorded. Packaged
release readiness remains future work.

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

No smoke tool is implemented. No readiness blocker is cleared. Packaged release
readiness remains future work.

P8AZ defines only the future report shape and redaction boundary for that
launcher-injected smoke evidence.

## Future Smoke Goal

A future smoke should prove:

```text
a launcher/package layer can invoke emuchef plan with a launcher-supplied absolute planner binary path through --rust-planner-bin <path>
```

The smoke should prove launcher injection without requiring the report to store
full local absolute paths.

## Required Smoke Assertions

A future smoke should assert:

1. the launcher supplies an absolute path;
2. the path resolves to an existing file;
3. the path is executable;
4. the observed Rust planner subprocess argv[0], after redaction, corresponds to the launcher-supplied planner binary path;
5. the plan command succeeds for a known fixture/device-plan route;
6. explicit `--planner-backend python` remains bypassable and reference-only;
7. no packaged helper lookup, `PATH`, env-var, Cargo, current-working-directory,
   or repo-local fallback is used.

## Redaction Rules

The smoke report must not record:

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

## Readiness Relationship

The future smoke may become readiness evidence only after a later phase
implements the smoke tool and the readiness gate accepts its report kind.

P8AY itself clears no blocker. Packaged release readiness remains future work.
