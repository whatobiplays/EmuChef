# Rust Production-Equivalent Live Probe Smoke

## Purpose

This document defines the evidence bar for a future production-equivalent live
probe smoke. It is a design note only; it does not implement a smoke tool, run
live ADB, or provide smoke evidence.
`docs/rust-default-route-mismatch-warning-parity.md` separately records the
P8AF evidence bar for future default-route mismatch-warning parity.
`docs/rust-production-equivalent-route-implementation-plan.md` separately
records the P8AG implementation plan for a future explicit
production-equivalent route.

## Evidence Bar

A future smoke may contribute to clearing `real_device_probing_not_cut_over`
only if it exercises a default-route or production-equivalent Rust-owned live
probe path.

P8Y and P8AA do not satisfy this bar. P8Y exercises a manual direct Rust shadow
binary route, and P8AA exercises an explicit `rust-experimental` migration
route. Those routes are useful migration evidence, but they are not default
planner or production-equivalent evidence.

## Required Route Characteristics

The future smoke route must:

- use Rust-owned probe execution;
- use Rust-owned detected-fact interpretation;
- follow the P8AD request/response boundary recorded in
  `docs/rust-default-route-probe-request-response.md`;
- preserve ADR 0002 output and exit-code compatibility;
- exercise the same route intended for default or production-equivalent planner
  use;
- avoid Python parsing of `adb shell getprop`;
- avoid Python reimplementation of Rust probe logic;
- keep explicit CLI context as an override over detected facts.

## Required Assertions

The future smoke should assert:

- a selected probe target is passed to Rust during transition, or otherwise
  selected by the production-equivalent route;
- Rust performs or owns live probe execution;
- detected facts affect `execution_plan.device_context`;
- explicit CLI context overrides detected facts when supplied;
- planner result output remains compatible with ADR 0002;
- no raw ADB output leaks into user-facing output;
- no raw serial, host paths, or environment details leak into deterministic
  reports;
- no Tauri/protocol or executor/apply behavior is implied unless explicitly
  scoped.

## Acceptable Inputs

Acceptable future inputs are conceptual:

- authored root;
- device plan;
- selected device/probe intent;
- optional ADB path;
- optional selected serial;
- optional explicit context overrides;
- bindings.

This document does not choose concrete CLI flags or a public schema.

## Acceptable Outputs

Acceptable future outputs are conceptual:

- `PlanningResult`-compatible route output;
- status `success` or `warning`;
- warnings and errors through the existing planner contract;
- `execution_plan.device_context` populated from detected facts plus overrides;
- deterministic scrubbed smoke report.

## Failure Classification

Future smoke failures should classify these conceptual failure classes:

- probe unavailable;
- probe failed;
- selected device/probe intent invalid;
- planner output incompatible;
- detected facts not reflected in `execution_plan.device_context`;
- explicit override precedence violated;
- sensitive output leaked.

This document does not freeze final CLI stderr strings or new exit-code behavior
beyond ADR 0002 compatibility.

## Non-Goals

P8AE does not:

- implement a smoke tool;
- run live ADB;
- implement or expose a production-equivalent backend;
- change the default planner route;
- change CLI, Rust, Python, test, or smoke source;
- modify readiness gate behavior;
- reclassify blockers;
- add Tauri/protocol or executor/apply integration;
- remove the Python planner.

## Blocker Implications

This design alone does not clear `real_device_probing_not_cut_over`.

A future smoke that satisfies this evidence bar may support reclassifying
`real_device_probing_not_cut_over`. It does not by itself clear
`detected_device_profile_mismatch_warning_not_cut_over` unless the same future
phase also proves default-route detected profile mismatch warning parity
satisfying `docs/rust-default-route-mismatch-warning-parity.md`.

After P8AE, both blockers remain blocked:

```text
real_device_probing_not_cut_over
detected_device_profile_mismatch_warning_not_cut_over
```
