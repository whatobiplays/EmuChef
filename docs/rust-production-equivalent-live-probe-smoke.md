# Rust Production-Equivalent Live Probe Smoke

## Purpose

This document defines the evidence bar for production-equivalent live probe
smoke evidence and records the optional/manual P8AJ smoke tool. P8AJ adds
`tools/smoke_rust_production_equivalent_live_adb_probe.py` as tooling only. The
tool can produce evidence when it is run manually with real device inputs, but
the tool's existence alone does not clear readiness blockers.
`docs/rust-default-route-mismatch-warning-parity.md` separately records the
P8AF evidence bar for future default-route mismatch-warning parity.
`docs/rust-production-equivalent-route-implementation-plan.md` separately
records the P8AG implementation plan and current state for the explicit
production-equivalent route.

## Evidence Bar

Manual P8AJ smoke output may contribute to future
`real_device_probing_not_cut_over` evidence only when it is produced by running
the tool against real device inputs through the explicit production-equivalent
route.

P8Y and P8AA do not satisfy this bar. P8Y exercises a manual direct Rust shadow
binary route, and P8AA exercises an explicit `rust-experimental` migration
route. Those routes are useful migration evidence, but they are not default
planner or production-equivalent evidence.

P8AJ targets this bar because it invokes:

```text
python -m emuchef plan --planner-backend rust-production-equivalent
```

with the Rust live-probe wrapper flags. P8AJ still does not make the smoke part
of normal checks or readiness-gate execution.

## Required Route Characteristics

The P8AJ smoke route must:

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

The smoke should assert:

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

P8AJ accepts:

- authored root;
- device plan;
- selected ADB path;
- selected serial;
- optional explicit context overrides;
- bindings.

The smoke report must scrub host paths and serials, must include
`live_probe_requested: true`, and must not include raw commands, stdout, stderr,
environment data, or device output.

## Acceptable Outputs

Acceptable smoke outputs are:

- `PlanningResult`-compatible route output;
- status `success` or `warning`;
- warnings and errors through the existing planner contract;
- Python-compatible concise output containing `Planning status: success` or
  `Planning status: warning`;
- Python-compatible YAML-like output containing `kind: planning_result` and
  `execution_plan:`;
- deterministic scrubbed smoke report.

Parseable raw Rust JSON stdout is incompatible for this smoke.

## Failure Classification

P8AJ smoke failures classify these stable failure classes:

- `production_equivalent_process_start_failed`;
- `production_equivalent_usage_failed`;
- `production_equivalent_unexpected_exit`;
- `production_equivalent_output_incompatible`;
- `adb_probe_unavailable`;
- `adb_probe_failed`;
- `stderr_text`.

Stable stderr markers `Error: adb_probe_unavailable` and
`Error: adb_probe_failed` map to the corresponding stable classifications
without copying raw stderr into the report.

## Non-Goals

P8AJ does not:

- run live ADB as part of normal checks;
- change the default planner route;
- change CLI, Rust backend, Python planner, executor, or apply behavior;
- modify readiness gate behavior;
- reclassify blockers;
- add Tauri/protocol or executor/apply integration;
- remove the Python planner.

## Blocker Implications

The P8AJ tool alone does not clear `real_device_probing_not_cut_over`.

Manual evidence produced by running the tool against real device inputs may
support a future reclassification of `real_device_probing_not_cut_over`. It does
not clear `detected_device_profile_mismatch_warning_not_cut_over`; that blocker
remains separate and still requires P8AK evidence satisfying
`docs/rust-default-route-mismatch-warning-parity.md`.

After P8AJ, both blockers remain blocked:

```text
real_device_probing_not_cut_over
detected_device_profile_mismatch_warning_not_cut_over
```
