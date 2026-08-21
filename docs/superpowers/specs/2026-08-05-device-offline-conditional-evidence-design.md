# Phase 6D.6 Conditional Device-Offline Evidence Design

## Decision

Keep `device_offline` as a supported runtime issue, physical harness scenario, evidence-schema value, and opportunistic evidence category, but remove it from the mandatory Phase 6D.6 closure matrix.

ADB `offline` is generally a transient transport state rather than a reliably operator-controlled condition. Qualification must not depend on racing device initialization or deliberately destabilizing the host ADB service. The mandatory physical transport proof remains the two USB-disconnect scenarios, which already cover active-operation and scheduling-boundary transport failure, authority invalidation, slot release, recovery, and fixture cleanup.

## Manifest model

The checked-in manifest distinguishes three sets:

- `scenarios`: every supported physical scenario accepted by the harness and validator;
- `mandatoryScenarios`: the twelve scenarios required for closure;
- `conditionalScenarios`: `device_offline` only.

`requiredRepetitions` remains `2`. Completeness therefore requires 24 mandatory physical repetitions plus the two composite UI-smoke repetitions.

## Validation behavior

- Evidence records for `device_offline` remain valid, sanitized, digest-checked, and auditable.
- Passing `device_offline` records do not reduce the mandatory missing count and are not required for `complete: true`.
- Gate validation continues to permit an explicitly selected `device_offline` run.
- The mandatory transport UI-smoke subcase must bind to passing `usb_disconnect_active` or `usb_disconnect_boundary` evidence. Conditional offline evidence cannot satisfy that closure binding.
- Runtime issue codes, executor behavior, Tauri projection, schema enums, and harness execution behavior do not change.

## Documentation

The runbook and current-state documentation describe `device_offline` as conditional diagnostic evidence. They prohibit relabeling disconnect, unauthorized, or ADB-server failures as offline evidence and state that inability to produce a stable offline transition does not block Phase 6D.6 closure.

## Verification

Regression coverage must prove:

1. the supported set remains thirteen scenarios;
2. the mandatory set contains twelve scenarios and excludes `device_offline`;
3. the conditional set contains only `device_offline`;
4. completeness is computed from the mandatory set only;
5. optional offline evidence remains valid and does not alter completeness;
6. transport UI binding rejects `device_offline` and accepts both USB-disconnect scenarios;
7. the checked-in manifest and runbook match the new contract.
