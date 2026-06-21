# Rust Default Planner Binary Resolution Design

## Purpose

This document records the P8AQ design contract for future Rust planner binary
resolution on the default Rust-owned `emuchef plan` route after P8AO, the P8AR
internal resolver foundation that centralizes current explicit-path validation,
the P8AS packaged binary location contract, and the P8AT inert packaged
resolver placeholder.

P8AQ is documentation-only. It does not change runtime CLI behavior, tests,
readiness-gate code, Rust backend code, smoke tools, executor/apply behavior,
Tauri/protocol behavior, packaging configuration, or Python planner deletion
behavior.

P8AR adds a behavior-preserving internal resolver helper in the Python CLI. It
supports only the explicitly supplied `--rust-planner-bin <path>` value and does
not add packaged lookup, Cargo fallback, `PATH` search, environment-variable
lookup, repo-local guessing, shell execution, or silent Python fallback.

P8AS is documentation-only. It records
`docs/rust-packaged-planner-binary-location.md` for the future packaged planner
binary location contract. It does not implement packaged lookup: the explicit
`--rust-planner-bin <path>` path remains required, explicit
`--planner-backend python` remains available, no Cargo fallback, `PATH` search,
env-var lookup, repo-local guessing, or silent Python fallback is implemented,
and packaged release readiness remains future work.

P8AT adds an inert packaged resolver placeholder in the Python CLI. The
placeholder currently returns `None`. No packaged lookup is implemented,
`--rust-planner-bin <path>` remains required, explicit `--planner-backend
python` remains available, no Cargo fallback, `PATH` search, env-var lookup,
repo-local guessing, or silent Python fallback is implemented, and packaged
release readiness remains future work.

P8AU adds tests for this packaged candidate seam only. The actual helper still
returns `None`, no packaged lookup is implemented, `--rust-planner-bin <path>`
remains required for real Rust routes, explicit `--planner-backend python`
remains available, no Cargo fallback, `PATH` search, env-var lookup, repo-local
guessing, or silent Python fallback is implemented, and packaged release
readiness remains future work.

P8AV tightens resolver error-contract tests only. It does not change runtime
behavior or implement packaged lookup; `--rust-planner-bin <path>` remains
required for real Rust routes, explicit `--planner-backend python` remains
available, and packaged release readiness remains future work.

P8AW records the packaged resolver implementation design in
`docs/rust-packaged-planner-resolver-implementation-design.md`. P8AW is
documentation-only. The proposed future mechanism is a
package/runtime-provided absolute path, once a later implementation defines the
integration point that supplies it. No packaged lookup is implemented,
`_packaged_rust_planner_bin_candidate(args)` still returns `None`,
`--rust-planner-bin <path>` remains required for real Rust routes, explicit
`--planner-backend python` remains available, no Cargo fallback, `PATH` search,
env-var lookup, repo-local guessing, or silent Python fallback exists, existing
Tauri sidecar/resource paths are not planner binary paths unless later
designated by a planner-specific decision, and packaged release readiness
remains future work.

P8AX is documentation-only. ADR 0005 accepts a launcher-supplied absolute path
through the existing `--rust-planner-bin <path>` option as the first packaged
planner integration path. No packaged lookup is implemented, no new CLI flag is
added, no env-var lookup is added, `_packaged_rust_planner_bin_candidate(args)`
still returns `None`, `--rust-planner-bin` remains required unless a launcher
supplies it, explicit `--planner-backend python` remains available, and
packaged release readiness remains future work.

P8AY is documentation-only and records
`docs/rust-launcher-injected-planner-smoke-contract.md` as the future smoke
evidence contract for launcher-injected `--rust-planner-bin <path>`. No smoke
tool is implemented, no packaged lookup is implemented, no readiness blocker is
cleared, `_packaged_rust_planner_bin_candidate(args)` still returns `None`,
`--rust-planner-bin` remains required unless a launcher supplies it, explicit
`--planner-backend python` remains available, and packaged release readiness
remains future work.

## Current State

P8AO made a no-backend `emuchef plan` invocation route through Rust-owned
planning by reusing the existing production-equivalent Rust subprocess path.

After P8AR, the default Rust route still requires an explicit
`--rust-planner-bin <path>` argument. Explicit `--planner-backend python`
remains available for the previous Python planning path.

No packaged binary lookup is implemented. No Cargo fallback is implemented. No
arbitrary `PATH` search is implemented. No environment-variable lookup is
implemented. No repo-local or host-path guessing is implemented. No silent
Python fallback is introduced when default Rust routing is active.

P8AS defines only the future packaged location contract. Existing Tauri sidecar
or resource packaging paths are not automatically planner-binary locations
unless a later planner-specific packaging decision explicitly designates them.

The Python CLI validates explicit Rust planner binary paths through an internal
resolver helper. That helper first applies Rust-route option compatibility
validation. If an explicit path is supplied, it expands the path, verifies that
it exists, verifies that it is a file, verifies that it is executable, and
returns that path for the subprocess argv. If no explicit path is supplied, the
resolver calls the inert P8AT packaged candidate helper once. The helper returns
`None`, so the resolver emits the same missing-bin error as before P8AT.

Executor/apply remains unresolved. Python planner deletion remains unresolved.
Packaged release readiness remains future work until a later implementation
proves binary lookup and bundling.

ADR 0005 defines the first packaged integration point as launcher injection
through the existing `--rust-planner-bin <path>` option. A later ADR may add a
true packaged resolver candidate branch if launcher injection proves
insufficient.

P8AY defines the future smoke evidence bar for that launcher-injected path. It
does not change the current explicit-path-only resolver behavior.

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

P8AQ, P8AR, and P8AT do not:

- implement packaged Rust planner binary lookup;
- bundle or package a Rust planner binary;
- search `PATH`;
- read environment variables for planner binary discovery;
- guess repo-local `target/debug` locations;
- run Cargo;
- execute through a shell;
- silently fall back to Python planning;
- delete the Python planner;
- change executor/apply behavior;
- change Tauri/protocol behavior;
- run ADB, planner smoke tests, Cargo, or readiness-gate checks;
- change any default-route runtime behavior.

## Proposed Resolution Order

Future default Rust planner binary resolution should use this order:

1. Use an explicit `--rust-planner-bin <path>` when supplied.
2. Use the package/runtime-provided absolute path after a later implementation
   defines the integration point that supplies it.
3. Fail with a deterministic error when no binary can be resolved.

Cargo build fallback must not be used by default runtime behavior.

Repo-local developer build lookup is not part of the packaged resolver contract
and must not mask missing packaged-binary defects.

## Error Contract

Future missing-binary errors should clearly distinguish these cases:

- explicit path missing;
- explicit path not executable;
- packaged path not configured;
- packaged binary missing;
- packaged binary not executable;
- packaged binary failed to start;
- packaged binary emitted invalid output.

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

P8AR tests the current explicit-path-only resolver foundation. A future packaged
lookup implementation should test:

- explicit `--rust-planner-bin <path>` still wins over packaged lookup;
- invalid explicit paths return explicit-path errors;
- non-executable explicit paths return explicit-path executable errors;
- package/runtime-provided paths are used only when explicit paths are absent;
- missing or non-executable packaged binaries return packaged-path errors;
- no shell, Cargo, arbitrary `PATH`, env-var lookup, repo-local, or host-path
  guessing fallback runs during normal runtime resolution;
- explicit `--planner-backend python` bypasses Rust binary resolution;
- default Rust routing fails deterministically when no binary can be resolved.

P8AT additionally tests that the inert packaged candidate helper returns `None`,
is consulted only when no explicit path is supplied, and is not consulted when
`--rust-planner-bin <path>` is present.

P8AY adds no tests. A later smoke implementation should use the P8AY contract to
prove launcher-supplied `--rust-planner-bin <path>` injection without requiring
reports to store full local absolute paths.

## Relationship To Readiness

P8AQ, P8AR, and P8AT do not change readiness-gate status. They do not
reclassify blockers or add accepted evidence.

`python_planner_deletion_not_ready` remains blocked.
`executor_apply_not_cut_over` remains blocked. Packaged release readiness
remains future work until an implementation proves binary lookup and bundling.
