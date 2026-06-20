# Rust Packaged Planner Binary Location Contract

## Purpose

This document records the P8AS contract for the future packaged/runtime
location of the Rust planner binary and the P8AT inert packaged resolver
placeholder.

P8AS is documentation-only. It does not change runtime CLI behavior, resolver
behavior, tests, readiness-gate logic, smoke tooling, Rust backend code,
executor/apply behavior, Tauri/protocol behavior, packaging configuration, or
Python planner deletion behavior.

## Current State

P8AO made a no-backend `emuchef plan` invocation route through Rust-owned
planning by reusing the existing production-equivalent Rust subprocess path.

P8AQ records the future binary-resolution design in
`docs/rust-default-planner-binary-resolution.md`.

P8AR adds an internal explicit-path resolver foundation for the current
`--rust-planner-bin <path>` validation.

After P8AS, the default Rust route still requires an explicit
`--rust-planner-bin <path>` argument. Explicit `--planner-backend python`
remains available for the previous Python planning path.

No packaged lookup is implemented. No Cargo fallback is implemented. No
arbitrary `PATH` search is implemented. No environment-variable or env-var
lookup is implemented. No repo-local guessing is implemented. No silent Python
fallback is implemented when Rust binary resolution fails.

Packaged release readiness remains future work.

P8AT adds an inert packaged resolver placeholder in the Python CLI. The
placeholder currently returns `None`. No packaged lookup is implemented,
`--rust-planner-bin <path>` remains required, explicit `--planner-backend
python` remains available, no Cargo fallback, `PATH` search, env-var lookup,
repo-local guessing, or silent Python fallback is implemented, and packaged
release readiness remains future work.

## Goals

- Define the future packaged Rust planner binary location contract.
- Avoid arbitrary host path guessing.
- Avoid `PATH` search.
- Avoid Cargo fallback.
- Avoid environment-variable discovery unless separately accepted.
- Provide deterministic future errors.
- Keep existing Tauri sidecar packaging evidence separate from future planner
  binary packaging decisions.

## Non-Goals

P8AS does not:

- implement packaged lookup;
- modify `src/emuchef/cli.py`;
- modify tests;
- change Tauri config;
- bundle or rename binaries;
- change readiness-gate behavior;
- delete the Python planner;
- change executor/apply behavior.

P8AT modifies only the Python CLI resolver seam, CLI tests, and current-state
documentation. It does not implement packaged lookup, choose a packaged path,
bundle binaries, rename binaries, change Tauri config, change readiness-gate
behavior, delete the Python planner, or change executor/apply behavior.

## Proposed Packaged Location Contract

The packaged app/runtime owns a configured planner-binary location. The CLI
resolver may use that packaged location only when the runtime explicitly
provides it.

The resolver must not infer the packaged location from an arbitrary current
working directory, `PATH`, Cargo output, or repository layout. Existing Tauri
sidecar or resource packaging paths are not automatically planner-binary
locations unless a later planner-specific packaging decision explicitly
designates them.

The actual concrete path mechanism must be selected in a later implementation
phase from one of these options:

1. package-provided absolute path
2. package-provided app resource directory plus known binary name
3. installer/config-generated resolver metadata

P8AS does not choose between those mechanisms because the current repository
does not yet define enough planner-specific packaging evidence.

## Binary Naming

The current development planner binary name is:

```text
emuchef-plan-shadow
```

The final packaged planner binary name may remain `emuchef-plan-shadow` or
change under a later explicit packaging decision. Any rename must update:

- Rust build/package scripts;
- Python resolver behavior;
- Tauri sidecar/resource config, if applicable;
- docs;
- tests.

## Error Contract

Future packaged lookup errors should distinguish:

- packaged location not configured;
- packaged binary missing;
- packaged binary not executable;
- packaged binary failed to start;
- packaged binary returned invalid output.

Errors should be deterministic and should identify the configured resolution
source and failing condition without retrying through Cargo, `PATH`,
environment-variable or env-var lookup, repo-local guessing, or silent Python
fallback.

## Relationship To Existing Resolver

P8AR resolver behavior remains explicit-path-only after P8AS. P8AT adds an
inert packaged candidate helper that currently returns `None`; it gives future
packaged lookup a named internal seam without changing current resolution
behavior.

A later implementation may add a packaged source branch to
`_resolve_rust_planner_bin`. Explicit `--rust-planner-bin <path>` must remain
the highest-priority source.

## Relationship To Readiness

P8AS does not change readiness status.

`executor_apply_not_cut_over` remains blocked.
`python_planner_deletion_not_ready` remains blocked.

Packaged release readiness remains unresolved until packaged lookup and bundling
are implemented and verified.
