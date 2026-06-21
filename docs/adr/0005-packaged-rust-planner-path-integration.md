# ADR 0005: Packaged Rust Planner Path Integration

## Status

Accepted

## Context

P8AW records the future packaged Rust planner resolver implementation design.
It selects a package/runtime-provided absolute path as the future mechanism, but
it does not define how that path enters the CLI process.

The current CLI resolver remains explicit-path-only. The existing
`--rust-planner-bin <path>` option is the validated path source for real Rust
planner routes, and `_packaged_rust_planner_bin_candidate(args)` still returns
`None`.

No packaged lookup is implemented. No new CLI flag is added. No env-var lookup
is added. Explicit `--planner-backend python` remains available for the
previous Python planning path. Packaged release readiness remains future work.

## Decision

This ADR accepts launcher injection of the existing
`--rust-planner-bin <path>` option as the first packaged planner integration
path. A later ADR may add a true packaged resolver candidate branch if launcher
injection proves insufficient.

The packaging/runtime launcher owns the concrete absolute planner-binary path
calculation. When the launcher starts the CLI process for a packaged runtime, it
may pass that absolute path through the existing `--rust-planner-bin <path>`
option. The CLI resolver then uses the already validated explicit-path branch.

The first packaged integration path does not need a hidden CLI flag, a new
public CLI flag, an environment variable, current-working-directory lookup, app
resource directory lookup, installer metadata lookup, or Tauri sidecar
inference.

## Rationale

1. The decision reuses the already validated explicit-path resolver branch.
2. Explicit `--rust-planner-bin <path>` remains the highest-priority source.
3. The resolver avoids adding a second path source before packaging has real
   evidence that it is needed.
4. The runtime avoids environment-variable ambiguity and implicit host-path
   guessing.
5. Existing Tauri sidecar/resource paths are not overloaded as planner binary
   paths.
6. Packaged runtime behavior stays deterministic because the launcher owns the
   absolute path calculation.

## Consequences

P8AX is documentation-only. No packaged lookup is implemented, no resolver code
changes, and `_packaged_rust_planner_bin_candidate(args)` still returns `None`.

No new CLI flag is added. No env-var lookup is added. `--rust-planner-bin`
remains required unless a launcher supplies it through the existing
`--rust-planner-bin <path>` option.

Explicit `--planner-backend python` remains available. No readiness blocker is
cleared by this ADR alone, and packaged release readiness remains future work.

Future packaged runtime can stop requiring user-authored `--rust-planner-bin`
arguments by having the launcher inject the existing option. The CLI resolver
does not need to infer packaged paths itself for the first packaged
integration.

A later ADR may add a true packaged resolver candidate branch only if launcher
injection is insufficient.

## Non-Goals

P8AX does not:

1. edit CLI resolver code;
2. edit tests;
3. add Tauri configuration;
4. add packaging scripts;
5. add smoke tools;
6. change readiness tooling;
7. change Rust backend code;
8. define an environment variable;
9. define a new CLI flag;
10. infer Tauri sidecar or resource paths as planner binary paths;
11. change executor/apply behavior;
12. change Python planner deletion readiness.
