# Rust Live Probe Evidence And Cutover Gap

## Purpose

This document records the live-probe evidence and remaining cutover gaps after
P8X-P8AA. It is a current-state summary for humans reading the Rust planner
cutover documentation.

`docs/adr/0004-default-route-live-probe-cutover-design.md` records that future
default-route live probing should be Rust-owned and that P8X-P8AA migration
evidence does not clear the default-route probing or mismatch-warning blockers.

This document is not a release checklist and is not a readiness-gate result.
Manual smoke evidence from P8Y and P8AA does not make Rust the default planner
backend, does not reclassify blockers, and does not add required manual
readiness evidence.

## Evidence Layers

| Slice | Evidence layer |
| --- | --- |
| P8V | pure getprop command/parser foundation |
| P8W | live ADB probe adapter foundation behind injectable runner |
| P8X | direct Rust shadow live probe mode |
| P8Y | optional/manual direct Rust shadow live probe smoke |
| P8Z | Python rust-experimental wrapper forwarding |
| P8AA | optional/manual Python rust-experimental route smoke |

## Route Matrix

| Route | Live probe support | Evidence | Production/default? | Notes |
| --- | --- | --- | --- | --- |
| default Python backend | no | existing Python planner/reference behavior | yes/current default | no Rust live probe |
| rust-shadow Python backend | no wrapper live probe | existing shadow/matrix evidence | no | rejects P8Z wrapper flags |
| direct emuchef-plan-shadow | yes | P8X/P8Y | no | dev/manual direct binary route |
| rust-experimental Python backend | yes | P8Z/P8AA | no | explicit non-default migration route |
| Tauri/protocol | no | not applicable | no | unchanged |
| executor/apply | no | not applicable | no | unchanged |

## What Is Proven

- Rust can model and run live `adb shell getprop` through the Rust shadow
  binary.
- Direct Rust shadow live probe mode can be manually smoked.
- Python `rust-experimental` can forward live-probe intent without invoking or
  parsing ADB in Python.
- Python route smoke can verify Python-compatible output for that explicit
  route.
- `device_profile_mismatch` warning can be accepted as route evidence when a
  selected live device intentionally differs from the authored plan.

## What Is Not Proven

P8X-P8AA do not prove any of the following:

- default `emuchef plan` Rust cutover;
- production-route live probing parity;
- default-route detected profile mismatch warning parity;
- Python planner deletion readiness;
- Tauri/protocol integration;
- executor/apply integration;
- normal-check readiness;
- release readiness.

## Remaining Blockers

`real_device_probing_not_cut_over` remains blocked because live probing exists
only in direct Rust shadow and explicit rust-experimental migration routes, not
the default production planner path.

`detected_device_profile_mismatch_warning_not_cut_over` remains blocked because
mismatch warning evidence exists in helper/manual migration routes, not as
default production-route parity.

## Next Cutover Candidates

- default-route probe request/response shape following ADR 0004
- production-route mismatch-warning parity evidence
- readiness-gate blocker reclassification only after production/default-route
  evidence
- eventual Python planner deletion plan
