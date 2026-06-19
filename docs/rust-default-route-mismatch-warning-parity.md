# Rust Default-Route Mismatch Warning Parity

## Purpose

This document defines the evidence bar for future default-route detected profile
mismatch warning parity.

This is a design note only. It does not implement warning parity, change runtime
behavior, or change blocker status.
`docs/rust-production-equivalent-route-implementation-plan.md` separately
records the P8AG implementation plan for a future explicit
production-equivalent route.
P8AK adds optional/manual fixture-backed smoke tooling for this evidence bar
through the explicit production-equivalent route. The tool can produce evidence
when run manually, but the tool's existence alone does not clear blocker status.

## Evidence Bar

Future evidence may contribute to clearing
`detected_device_profile_mismatch_warning_not_cut_over` only if it proves
mismatch warning behavior through a default-route or production-equivalent
Rust-owned path.

Helper and manual migration-route warnings from P8Q, P8R, P8X, P8Y, P8Z, and
P8AA do not satisfy this bar by themselves. They remain useful migration
evidence, but they are not default-route or production-equivalent warning
parity evidence.

Manual P8AK smoke output from
`tools/smoke_rust_production_equivalent_mismatch_warning.py` may contribute to
future `detected_device_profile_mismatch_warning_not_cut_over` evidence when it
is produced by running the tool through the explicit production-equivalent route.
P8AJ remains separate: it targets production-equivalent live probe evidence, not
fixture-backed mismatch-warning parity evidence.

## Required Route Characteristics

The future evidence route must:

- use Rust-owned detected-fact interpretation;
- use the default-route or production-equivalent planner path;
- evaluate detected facts against authored/profile criteria;
- emit mismatch warnings through the planner result contract;
- preserve ADR 0002 output and exit-code compatibility;
- preserve ADR 0004 context precedence;
- avoid Python reimplementation of warning logic.

## Required Assertions

Future evidence should assert:

- a matched device/profile scenario does not emit `device_profile_mismatch`;
- a mismatched detected manufacturer/profile scenario emits a mismatch warning;
- a mismatched detected model/profile regex scenario emits a mismatch warning;
- a detected Android version below the authored minimum emits a mismatch
  warning;
- a detected Android version meeting the authored minimum does not emit a
  mismatch warning solely due to Android version;
- explicit CLI overrides affect the final `execution_plan.device_context`
  according to ADR 0004 precedence;
- warning output remains compatible with the current planner result contract;
- no raw ADB output, serial, host paths, or environment details leak into
  deterministic reports.

## Match Criteria Scope

The current warning criteria surface is limited to:

- manufacturer;
- brand;
- model regex;
- Android minimum version.

Authored Android maximum values may be parsed, but they are not currently part
of warning evaluation unless a future accepted phase changes parity scope.

## Acceptable Inputs

Acceptable future inputs are conceptual:

- authored root;
- device plan;
- selected device/probe intent or fixture-equivalent detected facts;
- authored/profile criteria;
- optional explicit context overrides;
- bindings.

This document does not choose concrete CLI flags, fixture schema, or public wire
format.

## Acceptable Outputs

Acceptable future outputs are conceptual:

- `PlanningResult`-compatible route output;
- status `success` or `warning`;
- warnings through the existing planner contract;
- `device_profile_mismatch` or future accepted equivalent warning code;
- `execution_plan.device_context` reflecting detected facts plus overrides;
- deterministic scrubbed evidence report.

## Failure Classification

Future evidence failures should classify these conceptual failure classes:

- expected mismatch warning missing;
- unexpected mismatch warning present;
- match criterion not evaluated;
- override precedence violated;
- planner output incompatible;
- sensitive output leaked.

This document does not freeze final production stderr strings or new exit-code
behavior.

## Non-Goals

P8AF does not:

- implement mismatch warning parity;
- run live ADB;
- implement or expose a production-equivalent backend;
- add or change smoke tools;
- change the default planner route;
- change CLI, Rust, Python, test, or smoke source;
- modify readiness gate behavior;
- reclassify blockers;
- add Tauri/protocol or executor/apply integration;
- remove the Python planner.

## Blocker Implications

This design alone does not clear
`detected_device_profile_mismatch_warning_not_cut_over`.

P8AK tooling alone does not clear
`detected_device_profile_mismatch_warning_not_cut_over`. Manual evidence from
that tooling may support a future P8AL readiness-gate update only after the
evidence exists.

`real_device_probing_not_cut_over` remains separate unless the same future phase
also satisfies the P8AE production-equivalent live probe evidence bar.

After P8AF, both blockers remain blocked:

```text
real_device_probing_not_cut_over
detected_device_profile_mismatch_warning_not_cut_over
```
