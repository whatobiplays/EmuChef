# Rust Default Planner Cutover Contract

## Purpose

This document defines the default-backend cutover contract for changing
`emuchef plan` from Python-owned planning to Rust-owned planning.

P8AM performs no cutover. It does not change CLI behavior, planner routing,
readiness-gate logic, Rust backend code, smoke tooling, executor/apply behavior,
Tauri/protocol behavior, blocker status, or test coverage.

## Current State

P8AN is the evidence-preflight phase. The readiness gate accepts explicitly
supplied P8AJ and P8AK reports and can mark only the corresponding
production-equivalent evidence blockers as `evidence_accepted`.

P8AO changes no-backend `emuchef plan` to Rust-owned planner routing through
the existing production-equivalent Rust subprocess path. The default route
requires `--rust-planner-bin <path>` during this transition and emits
Python-compatible CLI output. Explicit `--planner-backend python` remains a
transition fallback and keeps the previous Python planning behavior.

`rust-production-equivalent` also remains available as an explicit backend. It
is a Rust-shadow-binary-backed route that can still be selected with
`--planner-backend rust-production-equivalent` and `--rust-planner-bin <path>`.

P8AL lets the readiness gate consume explicitly supplied P8AJ and P8AK reports.
Accepted reports can move only the relevant evidence-backed blocker entries to
`evidence_accepted`. P8AL `evidence_accepted` status does not clear default
cutover by itself, and the top-level readiness status remains `blocked`.

After P8AO, runtime behavior resolves the default-route ownership gap. P8AP
updates the static readiness gate to preserve the historical
`default_cli_backend_still_python` entry with status `resolved`. The remaining
non-planner-routing blockers include:

- `executor_apply_not_cut_over`
- `python_planner_deletion_not_ready`

P8AQ records the future binary-resolution design in
`docs/rust-default-planner-binary-resolution.md`. It is design-only:
`--rust-planner-bin <path>` remains required, no packaged binary lookup or Cargo
fallback is implemented, explicit `--planner-backend python` remains available,
and packaged release readiness remains future work.

## Cutover Preconditions And Evidence

P8AO depends on these preconditions before changing the default `emuchef plan`
route to Rust-owned planning:

1. A P8AJ live-probe report is accepted by the readiness gate.
2. A P8AK mismatch-warning report is accepted by the readiness gate.
3. ADR 0002 output and exit-code behavior are preserved.
4. ADR 0004 context precedence is preserved:
   authored/profile-derived context, then detected facts, then explicit CLI
   context overrides.
5. `rust-shadow` and `rust-experimental` do not regress.
6. The implementation scope states whether executor/apply remains Python-owned
   after the planner default-backend flip.
7. The implementation scope states that Python planner deletion is not part of
   the default-backend flip.

## Default Route Behavior Contract

After P8AO, a no-backend invocation of `emuchef plan`, meaning no
`--planner-backend` argument, routes through
Rust-owned planning.

The default route must preserve the Python-compatible CLI contract unless a
separate accepted breaking-change decision changes that target:

- concise output preserves current Python-compatible text behavior;
- `--verbose` preserves current structured YAML behavior;
- `--output` preserves current YAML file behavior;
- stdout, stderr, and exit-code behavior preserve ADR 0002;
- detected facts and device profile mismatch warnings are Rust-owned on the
  explicit production-equivalent evidence routes;
- explicit CLI context overrides are forwarded to Rust planning;
- raw Rust JSON is not emitted by default.

Rust-native JSON requires a separate explicit structured-output flag, such as a
future accepted `--format json`, before it can become user-facing output.

## Transitional Backend Contract

`rust-production-equivalent` remains available during the transition after the
default-backend cutover. It remains explicit and non-default when selected with
`--planner-backend rust-production-equivalent`.

`rust-shadow` remains a dev-only route. Its passthrough and explicit formatter
behavior must not change as a side effect of the default-backend flip.

`rust-experimental` remains a migration-only route. Its name and behavior may
change only under separate explicit scope.

Removing or renaming any planner backend requires separate explicit scope.

## Required Non-Regression Tests for P8AO

P8AM does not add tests. P8AO adds or updates tests that prove the
default-backend flip without relying on this contract document as evidence.

P8AO must prove:

- default no-backend invocation uses Rust-owned planning;
- default no-backend invocation requires `--rust-planner-bin` during the
  transition;
- `--planner-backend python` remains available and does not require
  `--rust-planner-bin`;
- `rust-shadow` behavior remains unchanged;
- `rust-experimental` behavior remains unchanged;
- explicit `rust-production-equivalent` behavior remains unchanged;
- concise output matches ADR 0002;
- `--verbose` YAML matches ADR 0002;
- `--output` YAML matches ADR 0002;
- explicit CLI context overrides are forwarded to Rust;
- `--rust-detected-facts-json` and live-probe wrapper flags remain explicitly
  scoped and are not accepted by no-backend default routing in P8AO.

## Non-Goals

P8AO does not:

- delete the Python planner;
- cut over executor/apply behavior;
- change Tauri/protocol behavior;
- reclassify readiness-gate blockers;
- run smoke tools;
- execute live ADB probing;
- change Rust backend implementation.

Python planner deletion is not part of the default-backend flip. Executor/apply
cutover is not part of the default-backend flip. Tauri/protocol changes are not
part of the default-backend flip.

## Implementation Slice

P8AN is the evidence-preflight phase. It verifies accepted P8AJ/P8AK evidence
reports without changing repo files.

P8AO is the default-route implementation phase. It may edit CLI routing, tests,
and current-state docs only after required P8AJ/P8AK evidence is accepted and
the implementation scope preserves the contracts in this document.

P8AP is readiness-gate classification cleanup only. It does not change runtime
CLI routing, smoke tooling, Rust backend behavior, executor/apply,
Tauri/protocol behavior, or Python planner deletion behavior.
