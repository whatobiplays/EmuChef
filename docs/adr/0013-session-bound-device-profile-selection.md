# ADR 0013: Session-bound Device Profile selection for overlapping matches

**Status:** Accepted product direction (2026-11-15); implementation and target-specific evidence pending.

## Context

Device Profiles express *compatibility criteria*, not uniquely attested retail identities. The KONKR Pocket Advance and AYANEO Pocket Air Mini may both match ADB-observed `ARBOR` / `GT78-VN` facts. Ranking one as the "best" match does not prove which physical product is connected. An incorrect profile can select unsafe storage locations, recommendations, Recipes or execution strategies, especially when saved intent or asynchronous responses outlive a device session.

## Decision

- Evaluate all candidate non-generic Device Profiles against the current trusted facts. **Any two or more matching non-generic profiles are ambiguous regardless of specificity, confidence or catalog order.** A single eligible non-generic profile may be selected automatically. Generic fallbacks are separate, do not manufacture ambiguity, and may be selected explicitly where their independently established capabilities suffice.
- With ambiguity, present an accessible **Choose Device Profile** step before **Choose Setup**. Show names dynamically from the catalog, with relevant observed facts and a clear explanation when ADB cannot distinguish the products. No option is preselected. Selecting a profile is the affirmative confirmation; there is no additional confirmation control or hardcoded model-specific action title.
- Treat the selected profile as a **user assertion**, not proof from ADB. Rust/Tauri retains the profile selection bound to the currently selected device, native session continuity and catalog identity. Planning, review and execution cannot gain authority from frontend state, cached recommendations or saved intent without the currently valid selection.
- Material device/firmware/profile-catalog changes that affect eligibility invalidate the confirmation and current review with an explanation. Equivalent refreshes do not. Reconnection with ambiguous candidates requires a fresh explicit choice; an earlier choice may be suggested, not automatically reactivated.
- Unknown facts needed for eligibility cannot establish a match. Offer retry or an independently eligible safe generic plan with the established limitations acknowledgment; do not force an unrelated model-specific profile.
- **Changing profiles starts a new setup.** Discard previous profile-specific settings, Setup Locations, descriptions and review, without cross-profile migration or compatibility adjudication. Block profile changes during active or uncertain execution until terminal verification/reconciliation; a later fresh setup does not roll back earlier mutations.
- Saved complete setups remain stored under their originating Device Profile and Device Plan. They can be reopened for that profile only after fresh session/eligibility checks and a new review; they do not migrate to a different selected profile. App-owned reusable templates remain separately selectable inputs for a new reviewed setup and are not automatic setup migration.
- Recommendation precedence (valid explicit choice, then model, brand, generic) applies **within an already selected profile**. Equal-specificity recommendation collisions remain governed by the existing recommendation contract.

## Rationale and rejected alternatives

The user cannot be expected to distinguish devices from internal Android model strings when the manufacturers themselves reuse those strings. Automatically picking the first, most-specific or highest-confidence matching profile would silently mistake compatibility for identity. A second post-selection confirmation adds no useful authority; the act of deliberately selecting a profile is sufficient. Carrying settings between different profiles would require a cross-device compatibility and migration subsystem, create stale-target hazards and complicate a guided setup. Starting a clean setup is the chosen simpler boundary.

## Consequences and invariants

The frontend and backend must distinguish a match result from a currently confirmed profile. Saved setups, reconnect/repair paths and `recommendedPlanId` cannot bypass ambiguity. Selection, review and execution must invalidate together on changed authority. This is a product-wide rule, not a KONKR-specific whitelist, a new formal support-status lifecycle, or a relaxation of operation-level qualification and production release gates.
