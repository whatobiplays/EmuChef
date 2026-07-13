# Phase 1 Read-Only End-User App

## 1. Product boundary

`apps/emuchef-app` is the separate end-user React/Tauri application. Its guided
workflow is Connect Device, Confirm Device, Choose Setup, Provide Inputs, and
Review Plan. Phase 1 is non-mutating: the application has no execution,
dry-run, apply, cancel, artifact download, saved-configuration, remote catalog,
wireless ADB, or multi-device execution surface.

This document defines the completed Phase 1 review boundary. The current app
adds simulated execution after Review as specified by
[Phase 2A Simulated End-User Execution](phase-2a-simulated-execution.md); that
addition does not change the Phase 1 discovery, configuration, or review
contracts described here.

The Rust sidecar launches and negotiates the `phase0_end_user_runtime`
extension plus the required read-only capabilities independently of ADB setup.
Runtime or catalog failure blocks the application. Missing ADB blocks only the
device workflow and presents the Platform-Tools setup flow.

## 2. Catalog and product operations

The MVP packages a verified catalog snapshot under the Tauri resource
directory. Startup requires the `apps`, `device_plans`, `device_profiles`, and
`recipes` directories, ignores only regular `.gitkeep` placeholders, rejects
symlinks and every other unsupported entry, and computes
the canonical snapshot SHA-256 before providing it to the sidecar. React sees
catalog identity and digest but no filesystem root. A cached remote catalog can
replace this materialized source later without changing planner or React DTOs;
Phase 1 implements no catalog networking.

The sidecar advertises and the Tauri backend negotiates these read-only product
operations:

1. `describeCatalog`
2. `listAdbDevices`
3. `probeDevice`
4. `matchDevice`
5. `describeConfiguration`
6. `planConfiguration`

No Phase 1 Tauri command invokes `startExecution`, `getExecution`,
`getExecutionEvents`, `cancelExecution`, CLI `apply`, or any device mutation.

## 3. Device discovery and matching

The sidecar owns ADB inventory parsing and exact device serials. Inventory
distinguishes no devices, unauthorized, offline, available, and multiple
devices. Tauri assigns a stable opaque handle to each serial while it remains
present in the current app session. React receives the handle, connection
state, a display label, and a masked serial only.

The backend probes manufacturer, brand, model, Android release, and API level,
then performs deterministic profile matching with `exact`, `high`, `low`, or
`none` confidence. A unique exact/high candidate may be recommended. Low or no
profile match is never auto-selected. The backend may return explicitly
approved safe generic plans; the user must choose one. The workflow blocks
only when the backend reports neither a candidate nor a safe generic plan.

Device disappearance invalidates its handle, retained facts, downstream
selection, and all target-bound reviews, then returns the frontend workflow to
the safe connection state.

## 4. Configuration and review

React does not implement recipe dependency, capability, input precedence, or
planner rules. `describeConfiguration` is the authority after meaningful
selection or input changes. It returns recommended/default recipes, optional
recipes, dependency-required recipes, unavailable capability reasons,
dependency-expanded selection, input declarations, effective values,
provenance, validation, and aggregate diagnostics. Native dialogs provide file
and directory values through non-blocking callbacks. Cancellation leaves the
current input unchanged and is not reported as an error. Field diagnostics are
rendered only beneath their corresponding input. Aggregate diagnostics remain
page-level only when they are not already represented by an input diagnostic;
identity uses binding key plus code when available and otherwise code plus
message.

The React request uses camelCase fields. A `null` `selectedRecipes` value means
the sidecar must expand the selected device plan's defaults; an explicit empty
array means the user selected no recipes. Probe facts remain snake_case inside
trusted inventory DTOs and Tauri converts them to the sidecar's camelCase
`deviceContext` and `targetDevice` fields. Structurally valid configurations
with missing required user inputs succeed and return `binding_missing`
diagnostics on the corresponding input descriptors.

Review generation is allowed only after the latest configuration description
has no blocking diagnostics or unresolved required values. The sidecar returns
a target-bound normalized plan and canonical digest. Tauri retains the full
immutable response, exact target binding, catalog identity/digest, and digest
behind an opaque review handle. React receives only a human-readable projection
grouped by recipe, selected input summaries, warnings, target facts without a
serial, action categories, capability requirements, and collapsed technical
step identifiers/types.

The Phase 1 implementation ended with a disabled future execution control.
Phase 2A replaces that placeholder with a simulation-only start action while
retaining the reviewed snapshot and trust boundary defined here.

## 5. Handle lifecycle and errors

Reviews are session-memory objects with these bounds:

1. maximum 16 live review snapshots;
2. 30-minute idle lifetime;
3. two-hour absolute lifetime; and
4. maximum 64 recent tombstones for stable stale/expired responses.

`review_stale` covers device disappearance, changed facts, catalog change,
Platform-Tools replacement/removal, explicit discard, and capacity eviction.
`review_expired` covers either time limit. `review_unknown` means the handle was
never known or its bounded tombstone has aged out. All cases require generating
a new review before future execution can be considered.

Tauri maps sidecar configuration failures to stable sanitized codes:
`configuration_request_invalid`, `configuration_catalog_invalid`,
`configuration_validation_failed`, or the fallback
`configuration_description_failed`. User messages contain recovery guidance,
not raw protocol data. Debug builds write the complete internal sidecar error to
the Rust terminal after redacting exact serials and absolute filesystem paths;
release builds do not emit raw internal errors.

## 6. Trust boundary

Sidecar/internal DTOs and React DTOs are separate types. Trusted Rust/Tauri
messages may carry exact serials, catalog roots, the managed ADB path, and the
full plan. React payloads, frontend state, storage, logs, and markup never carry
an exact serial, arbitrary ADB executable path, catalog root, or full plan.
Frontend paths selected as explicit recipe inputs are user-visible values and
are unrelated to the hidden managed ADB executable path.

The trusted `describeConfiguration` response retains its exact `targetDevice`
binding and verified catalog identity/digest. Tauri builds a separate
configuration projection containing only recipe options, input descriptors,
and sanitized diagnostics, so neither target serials nor catalog paths cross
the React boundary.
