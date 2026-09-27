---
name: edit-recipe
description: Edit an existing EmuChef authored recipe under authored/recipes, including targeted behavior changes and explicitly aligned recipe-ID migrations. Use when the user asks to change, fix, update, refine, rename, or migrate an existing recipe. Resolve natural-language targets against the catalog, preserve pre-existing working-tree edits and useful YAML shape, update authored references atomically for identity migrations, and optionally exercise the changed recipe through EmuChef's production execution path.
---

# Edit EmuChef Recipe

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Resolve the target from natural language against the current authored catalog. Ask only when multiple objects remain genuinely ambiguous; do not require an exact ID/path.
2. Use the current working-tree file as the baseline and preserve pre-existing edits.
3. Inspect current recipe schema, step/planner/runtime semantics, dependencies, references from plans/recipes, related app definitions, and relevant external evidence.
4. Make the requested semantic change plus only directly related corrections required to keep the recipe coherent. Report unrelated findings separately.
5. Preserve comments, ordering, and formatting where practical. Re-emit canonically only when authoritative tooling requires it.
6. If the ID changes, treat it as an explicitly aligned catalog migration: change the identity, rename the authored file when current repository convention couples filename to ID, and update every known authored reference under `authored/**` as one logical operation. Do not modify non-authored references; report them as engineering follow-up.
7. Align all material behavior/migration decisions and optional validation choices before writing.
8. Apply the approved authored edit and inspect the resulting diff.
9. Apply optional validation selected during alignment.
10. Record evidence and finish with the shared completion report.

## Optional physical validation

Offer the same optional real-device path as `create-recipe`: unambiguous suitable target, aligned mutations/inputs, EmuChef production execution only, no ad-hoc cleanup, meaningful postcondition verification, iterative repair within approved semantics, and environmental-failure classification without changing recipe behavior to fit the environment.
