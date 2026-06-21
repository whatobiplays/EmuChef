# Rust Packaged Planner Resolver Implementation Design

## Purpose

P8AW is documentation-only. It records the future packaged Rust planner
resolver implementation design without changing runtime CLI behavior, resolver
behavior, tests, readiness-gate logic, smoke tooling, Rust backend code,
executor/apply behavior, Tauri/protocol behavior, packaging configuration, or
Python planner deletion behavior.

The proposed future mechanism is a package/runtime-provided absolute path, once
a later implementation defines the integration point that supplies it.

P8AW does not implement packaged lookup. The current inert packaged resolver
helper still returns `None`, `--rust-planner-bin <path>` remains required for
real Rust routes, and explicit `--planner-backend python` remains available.

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

P8AZ is documentation-only and records
`docs/rust-launcher-injected-planner-smoke-report-schema.md` as the future
schema for `rust_launcher_injected_planner_smoke` reports with
`schema_version: 1`. No smoke tool is implemented, no readiness intake is
implemented, no readiness blocker is cleared, no resolver behavior changes, and
future reports must not record full local paths, serials, commands,
stdout/stderr, or environment variables. Packaged release readiness remains
future work.

## Current State

P8AO made a no-backend `emuchef plan` invocation route through Rust-owned
planning by reusing the existing production-equivalent Rust subprocess path.

P8AQ documented the future Rust planner binary resolution contract for the
default Rust-owned route.

P8AR added the internal explicit-path resolver foundation for the current
`--rust-planner-bin <path>` validation.

P8AS documented the packaged planner binary location contract without
implementing packaged lookup or designating existing Tauri sidecar/resource
paths as planner binary paths.

P8AT added `_packaged_rust_planner_bin_candidate(args)` as an inert packaged
candidate helper.

P8AU added seam tests for the inert packaged candidate helper.

P8AV tightened resolver error-contract tests without changing runtime behavior.

`_packaged_rust_planner_bin_candidate(args)` still returns `None`.
`--rust-planner-bin <path>` remains required today for real Rust routes.
Explicit `--planner-backend python` remains available for the previous Python
planning path.

No packaged lookup is implemented. No Cargo fallback is implemented. No `PATH`
search is implemented. No env-var lookup is implemented. No repo-local guessing
is implemented. No silent Python fallback is implemented when Rust binary
resolution fails.

Packaged release readiness remains future work.

## Proposed Future Mechanism

The proposed future mechanism is a package/runtime-provided absolute path, once
a later implementation defines the integration point that supplies it.

A later runtime implementation may use only an absolute path supplied by an
explicit package/runtime integration point. That integration point is not
defined by P8AW.

P8AX defines the first integration point as launcher injection through the
existing `--rust-planner-bin <path>` option. A later ADR may add a true packaged
resolver candidate branch if launcher injection proves insufficient.

P8AY defines the future smoke evidence bar for that launcher-injected path. It
does not change resolver order or introduce a packaged candidate branch.

P8AZ defines the future report shape and redaction denylist for that smoke
evidence. It does not add a report producer or readiness-gate consumer.

The package/runtime-provided absolute path is a planner-specific resolver input.
It is not implied by existing Tauri sidecar/resource paths, and it is not
derived from development build output or host environment discovery.

## Forbidden Inference Sources

The packaged Rust planner resolver must not infer a planner binary path from:

1. current working directory;
2. `PATH`;
3. Cargo output;
4. repository layout;
5. arbitrary environment variables;
6. Tauri sidecar or resource paths unless a later planner-specific decision
   explicitly designates them for planner use.

## Deferred Mechanisms

These mechanisms remain deferred:

1. package-provided app resource directory plus known binary name;
2. installer/config-generated resolver metadata.

Both require more packaging evidence and cross-platform app-layout decisions
than the current repository defines. P8AW does not approve either mechanism for
the packaged planner resolver implementation.

## Configuration Boundary

The package/runtime-provided absolute path must come from an explicit
runtime/package integration point defined later. P8AW does not introduce that
integration point.

P8AW does not name a specific environment variable. Generic env-var lookup
remains outside the current resolver contract unless separately approved.

Existing Tauri sidecar/resource paths are not planner binary paths unless a
later planner-specific packaging decision explicitly designates them for
planner use.

## Future Resolver Order

Future packaged Rust planner resolver order should remain:

1. explicit `--rust-planner-bin <path>`;
2. package/runtime-provided absolute path, if configured by the later
   runtime/package integration;
3. deterministic missing-bin error.

Explicit `--rust-planner-bin <path>` must remain the highest-priority source.
The resolver must not retry through Cargo, `PATH`, env-var lookup, repo-local
guessing, or silent Python fallback when packaged resolution is unavailable or
invalid.

## Future Error Contract

The future implementation should distinguish:

1. explicit path missing;
2. explicit path not executable;
3. packaged path not configured;
4. packaged path missing;
5. packaged path not executable;
6. packaged binary failed to start;
7. packaged binary emitted invalid output.

Errors should identify the selected resolution source and failing condition
without implying that a lower-priority fallback was attempted.

## Future Test Strategy

A later implementation should test:

1. explicit path wins over package/runtime-provided path;
2. package/runtime-provided path is used only when explicit path is absent;
3. package/runtime-provided missing path fails deterministically;
4. package/runtime-provided non-executable path fails deterministically;
5. no `PATH`, env-var, current-working-directory, repo-local, or Cargo fallback
   occurs;
6. explicit Python backend bypasses Rust resolver.
