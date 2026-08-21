# Phase 6D.6 Device-Unauthorized Boundary Qualification Design

## Decision

Change `device_unauthorized` from an active-operation physical scenario to a safe-boundary physical scenario.

The failed physical attempt on Android 11 proved that revoking USB debugging authorizations can clear stored trust without invalidating an already authenticated ADB transport. The active host `Push` therefore completed and the selected serial never entered `unauthorized`. Qualification must force a new authentication handshake instead of assuming revocation interrupts an established connection.

## Physical sequence

1. Observe the selected serial authorized before execution.
2. Execute the first reviewed fixture-owned operation successfully.
3. Emit `boundary-ready` after the first step finishes.
4. Wait into a later canonical second, revoke USB debugging authorizations on the selected device, and create `authorization-revoked` with exact `ack` content.
5. Disconnect and reconnect the same selected device to force a new handshake.
6. Observe the selected serial absent for a non-zero canonical interval and then present as exact state `unauthorized`; the harness creates `unauthorized-observed` only after capturing that transition.
7. Create `operator-action` only after `unauthorized-observed` exists.
8. Start the second reviewed operation; it must fail before mutation with `device_unauthorized`.
9. Emit `terminal-ready`.
10. Reauthorize the same device, prove its exact state is `device`, then create `cleanup-ready`.
11. Perform fixture-only cleanup and record a final authorized observation.

## Harness changes

- Keep `device_unauthorized` mandatory and keep its authorization-reset opt-in.
- Move `DeviceUnauthorized` from active-checkpoint classification to boundary-checkpoint classification.
- Remove it from exact active-process capture and active host-push stimulus preparation.
- Use the ordinary two-step reviewed plan and require the first step to remain completed.
- Retain the production execution-session slot lifecycle, authority invalidation, no-automatic-resume, cleanup, residual, run-scope, and sanitization requirements.
- Extend authorization evidence with a real selected-serial absence interval and reconnect chronology so the unauthorized row cannot be self-attested or sampled from the original session.
- Preserve the exact former active-scenario contract only as an approved legacy audit snapshot for non-passing records; it can never qualify a repetition.

## Evidence contract

A qualifying repetition has:

- `executionSuccess: false`;
- `observedIssueCode: device_unauthorized`;
- step states exactly `executed: 1`, `failed: 1`, `notAttempted: 0`;
- no required `activeProcess` evidence;
- initial authorized observation before the first operation;
- first operation completion before revocation;
- revocation checkpoint before serial absence;
- a non-empty serial-absent interval;
- same selected serial returning as `unauthorized` before or at terminal detection;
- terminal result before cleanup;
- final authorized observation after cleanup;
- authority invalidated, slot released, cleanup succeeded, and residual state clean.

The prior blocked active-attempt record and trace remain unchanged and auditable but do not count toward completeness.

## Scope boundaries

No runtime issue code, ADB classifier, Tauri projection, public API, retry behavior, execution semantics, schema version, or mandatory scenario count changes. The correction is limited to physical qualification choreography, evidence chronology, regressions, and documentation.
