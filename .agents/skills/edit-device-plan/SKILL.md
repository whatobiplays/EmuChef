---
name: edit-device-plan
description: Edit an existing EmuChef authored device plan under authored/device_plans, including plan identity/name/description, profile reference, recipe membership/default selection, active recipe-input overrides, metadata, and explicitly aligned plan-ID migrations. Use when changing a device's recommended setup or plan composition. Delegate semantic changes to referenced recipes/profiles/apps to their own edit skills and optionally validate the composed plan through EmuChef production execution.
---

# Edit EmuChef Device Plan

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Ownership

Own plan identity/name/description, `device_profile_ref`, recipe membership and default selection, active recipe-input overrides, and plan metadata. Delegate changes to referenced recipes, profiles, or app definitions to their specialized editors.

## Workflow

1. Resolve the target plan from natural language; ask only if multiple candidates remain ambiguous.
2. Use the working-tree file as the baseline and preserve existing edits.
3. Inspect current planner semantics, referenced profile/recipes/apps, capability compatibility, existing plans, and active override-binding rules.
4. Make the requested change plus directly related corrections required for a coherent plan. Report unrelated findings separately.
5. Treat recipe membership and `selected_by_default` changes as product decisions and include them in alignment.
6. Author overrides only according to current active planner semantics. Preserve comments/ordering where practical.
7. If the plan ID changes, align and apply one authored migration: identity, conventional filename rename, and any known authored references under `authored/**`. Report non-authored references without changing them.
8. Align the plan edit, delegated companion edits, and optional validation before writing.
9. Apply the approved authored change set and inspect the diff.
10. Run only explicitly selected validation, record evidence, and finish with the shared completion report.

## Optional physical validation

Offer end-to-end execution of the plan's default-selected recipes as a separate option. Use an unambiguous suitable device, aligned mutations/inputs, EmuChef production planning/review/execution, no ad-hoc cleanup, meaningful postcondition verification, iterative authored repair only within approved semantics, and environmental-failure classification without changing plan semantics merely to make the environment pass.
