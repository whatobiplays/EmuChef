# ADR 0012: RetroArch configuration-create policy boundary

**Status:** Accepted

## Context

RetroArch configuration creation is authorized from a trusted planned operation and its resolved App Target. Parsing a plan is a separate responsibility from retaining mutable root and device-session authority.

## Decision

`retroarch_config_create_policy` resolves the selected Recipe, configuration source, frozen artifact, App Target, package, destination, catalog identity, and create-only semantics. It rejects missing, malformed, ambiguous, or pruned configuration operations.

`RootQualificationStore` owns root attempts and routes, current session authority, destination-scoped write evidence, and evidence invalidation. Planning and execution resolve the same target identity before consulting that store. The store's evidence insertion helper remains test-only and is not exposed through IPC.

Existing rooted operations retain their profile restrictions, verified root route, session checks, execution guards, and legacy `appDataWrite` behavior. Managed or uploaded RetroArch configuration creation requires separate current evidence for its exact package, catalog, device session, root authority, and destination. The planner grants scoped `app_data_write` only to the create-only operation resolved against the RetroArch `config` App Target; matching the destination alone does not grant another step permission. Explicit `none` leaves provisioning to other targets outside this requirement. A copy that still resolves to `files/retroarch.cfg` remains subject to configuration-create evidence.

## Rationale

Keeping plan interpretation in a pure policy module makes its parser rules and failures independently testable. Keeping mutable authority in the root store gives a future trusted probe a single destination-scoped evidence boundary and keeps configuration parsing independent from probe mechanics.

A universal write probe would change admission for established rooted workflows. The policy therefore preserves their current authorization behavior and adds the stricter evidence requirement only to RetroArch configuration creation.

## Consequences

Root access and legacy `appDataWrite` do not authorize RetroArch configuration creation. The production probe, trusted evidence bridge, and KONKR device-session binding remain unimplemented, so physical configuration creation stays fail-closed until those boundaries are implemented and reviewed.
