---
name: create-device-plan
description: Create a new EmuChef authored device plan under authored/device_plans from high-level setup intent such as a recommended configuration for a supported handheld. Use when composing a device profile with recipes, choosing recipe membership/default selection, or adding plan-level input overrides and metadata. Inspect current planner semantics and the catalog, surface all composition decisions for alignment, and delegate missing profiles/recipes/apps to their specialized create/edit skills.
---

# Create EmuChef Device Plan

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Resolve the high-level setup intent and target device/profile.
2. Inspect the current device-plan parser/planner semantics, target device profile, available recipes, relevant app definitions, existing plans, and capability relationships.
3. Determine the minimal coherent recipe composition. Compatibility is a fact; whether a recipe belongs in the plan and whether it is selected by default are product decisions.
4. Present every recipe-membership and `selected_by_default` decision during alignment.
5. Author plan overrides only when they are needed to express the intended setup and are grounded in the referenced recipe's current input contract. Follow actual planner semantics; never treat an inactive or metadata-only field as runtime binding authority.
6. If required profiles, recipes, or app definitions are missing, include them in the aligned authored-object set and delegate them to their specialized skills.
7. Detect duplicate plan identity/collisions before writing. If the same logical plan exists, explain the match and ask whether to switch to `edit-device-plan`.
8. Write only the new device plan under `authored/device_plans/**` from this skill.
9. Apply optional validation selected during alignment.
10. Record evidence and finish with the shared completion report.

## Optional physical validation

Offer end-to-end execution of the plan's default-selected recipes as a separate optional choice. When selected, use the same production-execution rules as `create-recipe`: unambiguous suitable target, aligned mutations and required inputs, EmuChef production planning/review/execution, no ad-hoc cleanup, verified meaningful postconditions, iterative repair only within approved semantics, and no authored changes in response to environmental failures.
