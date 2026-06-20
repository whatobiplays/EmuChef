# Rust Default Planner Binary Resolution Design

## Purpose

This document records the P8AQ design contract for future Rust planner binary
resolution on the default Rust-owned `emuchef plan` route after P8AO.

P8AQ is documentation-only. It does not change runtime CLI behavior, tests,
readiness-gate code, Rust backend code, smoke tools, executor/apply behavior,
Tauri/protocol behavior, packaging configuration, or Python planner deletion
behavior.

## Current State

P8AO made a no-backend `emuchef plan` invocation route through Rust-owned
planning by reusing the existing production-equivalent Rust subprocess path.

After P8AQ, the default Rust route still requires an explicit
`--rust-planner-bin <path>` argument. Explicit `--planner-backend python`
remains available for the previous Python planning path.

No packaged binary lookup is implemented. No Cargo fallback is implemented. No
arbitrary `PATH` search is implemented. No host-path guessing is implemented.
No silent Python fallback is introduced when default Rust routing is active.

Executor/apply remains unresolved. Python planner deletion remains unresolved.
Packaged release readiness remains future work until a later implementation
proves binary lookup and bundling.

## Goals

- Define future Rust planner binary lookup behavior for default Rust-owned
  planning.
- Preserve explicit `--rust-planner-bin <path>` as the highest-priority
  override.
- Avoid Cargo fallback in normal runtime behavior.
- Avoid host-path guessing that hides packaging defects.
- Keep missing-binary and non-executable-binary errors deterministic and
  actionable.

## Non-Goals

P8AQ does not:

- implement Rust planner binary lookup;
- bundle or package a Rust planner binary;
- delete the Python planner;
- change executor/apply behavior;
- change Tauri/protocol behavior;
- run ADB, planner smoke tests, Cargo, or readiness-gate checks;
- change any default-route runtime behavior.

## Proposed Resolution Order

Future default Rust planner binary resolution should use this order:

1. Use an explicit `--rust-planner-bin <path>` when supplied.
2. Use the packaged or bundled planner binary location after packaging defines
   that location.
3. Use a repo-local developer build path only under a separately accepted
   explicit dev-mode mechanism.
4. Fail with a deterministic error when no binary can be resolved.

Cargo build fallback must not be used by default runtime behavior.

Repo-local developer build lookup, if added later, must be gated behind an
explicit dev-mode mechanism. It must not run in packaged/runtime mode and must
not mask missing packaged-binary defects.

Packaged/runtime mode means an installed, bundled, or release-style invocation.
Developer source checkout behavior must stay opt-in so local build artifacts do
not hide release packaging defects.

## Error Contract

Future missing-binary errors should clearly distinguish these cases:

- explicit path missing;
- explicit path not executable;
- packaged binary missing;
- packaged binary not executable;
- no resolver configured.

Errors should identify the selected resolution source and the failing condition
without leaking unrelated host details. They should not silently retry through a
lower-priority source when doing so would hide a packaging defect.

## Security And Reproducibility

Default Rust planner binary resolution should not search arbitrary `PATH`
entries. It should not execute through a shell. It should not infer binary
locations from environment variables unless a separate accepted design approves
that behavior.

The resolver should not run Cargo implicitly. Cargo may remain a developer
build tool, but default runtime planning must execute a resolved binary rather
than build one on demand.

Default Rust planner routing must not silently fall back to Python when Rust
binary resolution fails. Explicit `--planner-backend python` remains available
as the intentional Python fallback path.

## Future Testing Strategy

A future implementation should test:

- explicit `--rust-planner-bin <path>` still wins over packaged lookup;
- invalid explicit paths return explicit-path errors;
- non-executable explicit paths return explicit-path executable errors;
- packaged paths are used only when configured by the packaged resolver;
- missing or non-executable packaged binaries return packaged-path errors;
- no shell, Cargo, arbitrary `PATH`, or host-path guessing fallback runs during
  normal runtime resolution;
- explicit `--planner-backend python` bypasses Rust binary resolution;
- default Rust routing fails deterministically when no binary can be resolved.

## Relationship To Readiness

P8AQ does not change readiness-gate status. It does not reclassify blockers or
add accepted evidence.

`python_planner_deletion_not_ready` remains blocked.
`executor_apply_not_cut_over` remains blocked. Packaged release readiness
remains future work until an implementation proves binary lookup and bundling.
