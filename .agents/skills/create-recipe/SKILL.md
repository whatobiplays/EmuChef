---
name: create-recipe
description: Create a new EmuChef authored recipe under authored/recipes from either a complete procedure or a higher-level app/feature/setup outcome. Use when the user asks to add a recipe, install/provision/configure/copy/sync behavior, or new reusable authored workflow. Research missing technical facts, compose existing catalog behavior where possible, and produce a production-ready recipe without changing EmuChef code. If the logical recipe already exists, offer a handoff to edit-recipe instead of creating a duplicate.
---

# Create EmuChef Recipe

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Resolve the requested outcome. Accept either a fully specified procedure or high-level intent.
2. Inspect the current recipe model, step specifications, planner/runtime semantics, existing recipes, app definitions, device plans, and validation/generation paths that bear on the request. Treat the current repository as schema authority.
3. Search for existing recipes/features/dependencies that already provide part or all of the requested behavior. Compose rather than duplicate.
4. Research missing technical facts when needed. Prefer upstream project documentation, repositories/releases, APK inspection, and other primary sources.
5. Determine whether EmuChef's current authored recipe primitives can express the requested behavior. If not, stop with the exact missing runtime/schema capability; never change code from this skill.
6. Detect logical duplicates and blocking ID/path/catalog collisions before writing. If the requested recipe already exists, explain why it is the same logical object and ask whether to switch to `edit-recipe`.
7. Build a production-ready proposed recipe contract. Consider every applicable concern: dependencies, provided features, inputs, artifacts, artifact groups, step dependencies, capabilities, conflicts, skip behavior, toggleability, failure semantics, verification, progress/user-facing metadata, and externally visible postconditions.
8. Bring only material behavioral choices to alignment. Typical choices include merge/replace/sync semantics, optional versus required work, dependency boundaries, device paths, artifact/version policy, verification expectations, and failure behavior.
9. Include any required companion authored objects in the aligned change set and delegate them to their own create/edit skills.
10. Write only the approved new recipe in `authored/recipes/**` from this skill.
11. Apply optional validation selected during alignment.
12. Record evidence and finish with the shared completion report.

## Optional physical validation

Offer real-device execution as a separate optional validation choice.

When selected:

- choose a target only when EmuChef can confidently identify an applicable profile/plan and probed capabilities satisfy the recipe;
- if zero or multiple suitable devices exist, return target selection to the user rather than guessing;
- include the chosen target and concrete expected mutations in alignment; after that approval, do not ask for a redundant second confirmation;
- execute through EmuChef's production planning/review/execution boundary, never through an ad-hoc direct-ADB substitute;
- request required user-owned inputs during alignment; missing inputs may leave physical validation incomplete without blocking creation of an otherwise valid recipe;
- do not uninstall, delete, restore, or otherwise clean up after execution unless that behavior is itself part of the recipe under test;
- count physical qualification as successful only when EmuChef execution succeeds and meaningful externally observable outcomes are verified wherever the current recipe model supports verification.

If execution reveals an authored defect, repair and rerun automatically when the correction stays within the already-approved semantics. Return to alignment for new behavior, broader mutation, or another product decision. Treat environmental failures as environmental; do not mutate recipe semantics merely to make the environment pass.
