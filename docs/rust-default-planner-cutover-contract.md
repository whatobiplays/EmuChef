# Rust Default Planner Cutover Contract

## Purpose

This document defines the future default-backend cutover contract for changing
`emuchef plan` from Python-owned planning to Rust-owned planning in a later
phase.

P8AM performs no cutover. It does not change CLI behavior, planner routing,
readiness-gate logic, Rust backend code, smoke tooling, executor/apply behavior,
Tauri/protocol behavior, blocker status, or test coverage.

## Current State

Default `emuchef plan` remains Python-owned.
`rust-production-equivalent` remains explicit and non-default. It is a
Rust-shadow-binary-backed route that must be selected with
`--planner-backend rust-production-equivalent` and `--rust-planner-bin <path>`.

P8AL lets the readiness gate consume explicitly supplied P8AJ and P8AK reports.
Accepted reports can move only the relevant evidence-backed blocker entries to
`evidence_accepted`. P8AL `evidence_accepted` status does not clear default
cutover, and the top-level readiness status remains `blocked`.

The remaining default-cutover blockers include:

- `default_cli_backend_still_python`
- `executor_apply_not_cut_over`
- `python_planner_deletion_not_ready`

## Cutover Preconditions

A later phase must satisfy all of these preconditions before changing the
default `emuchef plan` route to Rust-owned planning:

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

After a future default-backend cutover, a no-backend invocation of
`emuchef plan`, meaning no `--planner-backend` argument, must route through
Rust-owned planning.

The future default route must preserve the current Python-compatible CLI
contract unless a separate accepted breaking-change decision changes that
target:

- concise output preserves current Python-compatible text behavior;
- `--verbose` preserves current structured YAML behavior;
- `--output` preserves current YAML file behavior;
- stdout, stderr, and exit-code behavior preserve ADR 0002;
- detected facts and device profile mismatch warnings are Rust-owned;
- explicit CLI context overrides detected facts according to ADR 0004;
- raw Rust JSON is not emitted by default.

Rust-native JSON requires a separate explicit structured-output flag, such as a
future accepted `--format json`, before it can become user-facing output.

## Transitional Backend Contract

`rust-production-equivalent` may remain available during the transition after a
future default-backend cutover. It remains explicit and non-default until a
separate phase changes or retires it.

`rust-shadow` remains a dev-only route. Its passthrough and explicit formatter
behavior must not change as a side effect of the default-backend flip.

`rust-experimental` remains a migration-only route. Its name and behavior may
change only under separate explicit scope.

Removing or renaming any planner backend requires separate explicit scope.

## Required Non-Regression Tests for P8AN

P8AM does not add tests. The future P8AN implementation must add or update tests
that prove the default-backend flip without relying on this contract document as
evidence.

P8AN must prove:

- default no-backend invocation uses Rust-owned planning;
- `--planner-backend python` remains available or is intentionally removed by
  explicit scope;
- `rust-shadow` behavior remains unchanged;
- `rust-experimental` behavior remains unchanged;
- `rust-production-equivalent` behavior remains unchanged or is explicitly
  retired;
- concise output matches ADR 0002;
- `--verbose` YAML matches ADR 0002;
- `--output` YAML matches ADR 0002;
- explicit CLI context overrides detected facts;
- `device_profile_mismatch` warning parity remains intact.

## Non-Goals

P8AM does not:

- delete the Python planner;
- cut over executor/apply behavior;
- change Tauri/protocol behavior;
- reclassify readiness-gate blockers;
- run smoke tools;
- execute live ADB probing;
- change Rust backend implementation;
- change CLI tests or runtime tests.

Python planner deletion is not part of the default-backend flip. Executor/apply
cutover is not part of the default-backend flip. Tauri/protocol changes are not
part of the default-backend flip.

## Future Implementation Slice

P8AN is the first possible default-route implementation phase.

P8AN may edit CLI routing and tests only after the required P8AJ/P8AK evidence
is accepted and the implementation scope preserves the contracts in this
document. P8AM must not make those changes.
