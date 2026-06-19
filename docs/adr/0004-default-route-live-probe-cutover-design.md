# ADR 0004: Default-Route Live Probe Cutover Design

Status: Accepted

## Context

Python is still the default CLI/reference planner owner. Rust planner live-probe
evidence exists only in direct Rust shadow and explicit `rust-experimental`
migration routes, not in the default production planner route.

P8AB records the current live-probe evidence and cutover gaps in
`docs/rust-live-probe-evidence-and-cutover-gap.md`. That evidence includes
manual direct-shadow and Python-wrapper smokes, but those smokes are evidence
only. They do not prove production/default readiness.

ADR 0003 records that Rust should own real-device probing and detected-device
profile mismatch warning parity for future default Rust planner cutover. The
remaining readiness blockers stay blocked:

- `real_device_probing_not_cut_over`
- `detected_device_profile_mismatch_warning_not_cut_over`

## Decision

The future default Rust planner route should own live ADB probing in Rust, not
Python. Python must not parse `adb shell getprop` output for the future default
Rust route, and Python must not reimplement Rust probe logic.

If a Python wrapper still exists during transition, it may pass selected
device/probe intent to Rust. Rust owns probe execution and detected-fact
interpretation.

The default-route cutover must preserve the output and exit-code contract
accepted in ADR 0002 unless another accepted ADR explicitly changes that
contract.

`docs/rust-default-route-probe-request-response.md` records the intended future
default-route probe request/response shape. It does not implement probing or
clear readiness blockers.
`docs/rust-production-equivalent-live-probe-smoke.md` records the evidence bar
for a future production-equivalent live probe smoke. It does not implement a
smoke or clear readiness blockers.

Default-route context precedence is:

1. authored/profile-derived context is the fallback base;
2. detected facts establish live-device context;
3. explicit CLI context overrides detected facts.

This preserves the existing intended precedence and does not change runtime
behavior in P8AC.

Detected profile mismatch warnings must be emitted from the default production
route, not only helper/manual migration routes, before
`detected_device_profile_mismatch_warning_not_cut_over` can be cleared.

Live probing must be exercised through default-route or production-equivalent
evidence before `real_device_probing_not_cut_over` can be cleared.

## Consequences

P8X-P8AA remain migration evidence, not cutover completion. Future work should
move from explicit migration routes toward a production/default route.

The readiness blockers remain blocked until production/default-route evidence
exists. The readiness gate should not be reclassified merely because manual
migration smokes pass.

Tauri/protocol and executor/apply remain separate concerns unless a future
phase explicitly scopes them.

## Non-Goals

P8AC does not:

1. change default `emuchef plan`;
2. wire live probing into the default Python or Rust route;
3. remove the Python planner;
4. add Tauri/protocol integration;
5. add executor/apply integration;
6. modify readiness gate behavior;
7. add or run smoke tools;
8. change CLI output contracts.

## Future Work

1. P8AD default-route probe request/response shape.
2. Future implementation of a production-equivalent live probe smoke that
   satisfies the P8AE evidence bar.
3. P8AF default-route mismatch-warning parity evidence.
4. P8AG readiness blocker reclassification after production/default evidence.
5. Python planner deletion plan after default Rust route ownership is proven.
