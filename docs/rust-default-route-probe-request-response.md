# Rust Default-Route Probe Request/Response Shape

## Purpose

This document defines the intended future default-route live probe boundary
after the Rust-owned cutover design accepted in
`docs/adr/0004-default-route-live-probe-cutover-design.md`.

This is a design note only. It is not an implementation plan, not a release
checklist, and not readiness-gate evidence. It does not change runtime behavior
or clear any Rust planner cutover blocker.

## Design Inputs

This shape follows the current accepted cutover documentation and migration
evidence:

- `docs/adr/0002-rust-planner-cli-output-compatibility.md` records that future
  default Rust planner routing must preserve the current Python-owned
  `emuchef plan` output and exit-code contract unless another accepted ADR
  changes that contract.
- `docs/adr/0003-rust-real-device-context-ownership.md` records that Rust should
  own real-device probing and detected-device profile mismatch warning parity
  before future default Rust planner cutover.
- `docs/adr/0004-default-route-live-probe-cutover-design.md` records that future
  default-route live probing should be Rust-owned, with Python limited to
  passing selected device/probe intent during a transition if a Python wrapper
  still exists.
- `docs/rust-live-probe-evidence-and-cutover-gap.md` records the P8X-P8AA
  live-probe evidence and the remaining default-route and production-route gaps.
- `docs/rust-production-equivalent-live-probe-smoke.md` records the evidence
  bar for a future production-equivalent live probe smoke.
- `docs/rust-default-route-mismatch-warning-parity.md` records the evidence bar
  for future default-route mismatch-warning parity.
- `docs/rust-production-equivalent-route-implementation-plan.md` records the
  P8AG implementation plan for a future explicit production-equivalent route.
- The P8V, P8W, and P8X Rust probe foundations model ADB getprop command shape,
  parse detected facts, and wire live probing only into the direct dev-only Rust
  shadow binary.

## Request Shape

The future default-route probe request is conceptual input to a Rust-owned
planner/probe boundary. It should carry enough information for Rust to load the
authored plan inputs, select the intended probe target, execute live probing,
apply explicit caller context, and preserve the selected output compatibility
mode.

The conceptual request contains:

- `authored_root`
- `device_plan_ref`
- selected device/probe intent
- optional ADB path
- optional selected serial
- explicit context overrides:
  - manufacturer
  - model
  - android_version
  - device_tags
- bindings
- output mode or compatibility mode

"Selected device/probe intent" is intentionally conceptual here; P8AD does not
choose a concrete CLI flag set, JSON request schema, or Tauri protocol shape.

Python may pass selected device/probe intent during a transition. Python must
not parse `adb shell getprop`, interpret detected facts, or reimplement Rust
probe logic for the future default Rust route. Rust owns live probe execution
and detected-fact interpretation.

This document does not define a stable public JSON schema. A concrete wire
shape, if needed, belongs in a future implementation slice or accepted ADR.

## Response Shape

The future default-route probe response is the existing planner result contract:

- `PlanningResult`
- `status`
- `warnings`
- `errors`
- `execution_plan`
- `execution_plan.device_context`

Default-route output must preserve the ADR 0002 output and exit-code
compatibility target unless another accepted ADR supersedes that contract.

This document does not add a Rust-native default JSON output mode. Rust-native
JSON remains a separate future explicit structured-output decision.

## Context Precedence

Default-route context precedence remains the order accepted in ADR 0004:

1. authored/profile-derived context is the fallback base;
2. detected facts establish live-device context;
3. explicit CLI context overrides detected facts.

This document records the intended boundary shape only. It does not change
current runtime behavior, route behavior, planner ownership, or context
precedence in any existing command.

## Warning Semantics

The future default production route must emit detected profile mismatch warnings
before this blocker can be cleared:

```text
detected_device_profile_mismatch_warning_not_cut_over
```

`device_profile_mismatch` is the current warning code used in migration
evidence. This document does not freeze a new warning schema beyond the current
evidence and existing planner result contract.
`docs/rust-default-route-mismatch-warning-parity.md` records the evidence bar
for future default-route mismatch-warning parity.

## Error Semantics

The future default-route boundary should distinguish these conceptual error
classes:

- probe unavailable
- probe failed
- invalid or unavailable selected device/probe intent
- planner input invalid

Final CLI stderr and exit behavior must preserve ADR 0002 compatibility or
receive a later accepted ADR if changed. This document does not specify raw
stderr text as a stable production contract.

## Out of Scope

P8AD does not:

- implement default-route probing;
- change `emuchef plan`;
- change Python or Rust source;
- change smoke tools;
- change readiness gate behavior;
- reclassify blockers;
- add Tauri/protocol integration;
- add executor/apply integration;
- remove Python planner.

## Cutover Implications

This design note helps future implementation phases by naming the intended
request/response boundary for Rust-owned default-route live probing. It does not
provide production/default-route evidence and does not clear readiness blockers.
P8AE separately records the evidence bar a future production-equivalent live
probe smoke must satisfy.
P8AF separately records the evidence bar future default-route mismatch-warning
parity must satisfy.
P8AG separately records the recommended future implementation path for an
explicit production-equivalent route; it does not make that route available.

These blockers remain blocked:

```text
real_device_probing_not_cut_over
detected_device_profile_mismatch_warning_not_cut_over
```
