# Ownership, composition, collisions, and migrations

## Ownership

- `create-recipe` / `edit-recipe`: `authored/recipes/**`
- `create-device-profile` / `edit-device-profile`: `authored/device_profiles/**`
- `create-app-definition` / `edit-app-definition`: `authored/apps/**`
- `create-device-plan` / `edit-device-plan`: `authored/device_plans/**`
- `diagnose-authored-object`: read-only until a repair is aligned, then delegate mutation to the appropriate editor.

A specialized skill may orchestrate another specialized skill but must not absorb the other object's semantic ownership.

## Multi-object composition

For a broad request such as "add support for X":

1. Inspect the current catalog and product model.
2. Propose the minimal coherent authored-object set.
3. Align that complete set before writes.
4. Delegate each object to its owning skill.
5. Apply the set as one logical catalog operation.

If a new unresolved decision appears after alignment, pause that branch. Continue independent approved branches only when they cannot leave the catalog partially coherent.

## Create collisions

Creation is new-object-only. Search by identity, destination, references, and logical purpose before writing.

When an existing object appears to represent the same logical object:

1. explain the evidence for equivalence;
2. do not invent a suffixed ID/path;
3. ask whether to switch to the matching edit skill.

Treat blocking ID/path/catalog collisions as alignment decisions.

## ID migrations

ID changes are allowed only after explicit alignment. Treat them as catalog migrations, not field edits.

Within one logical authored migration:

1. change the object's ID;
2. rename its authored file when current repository convention couples filename to ID;
3. discover and update all known references under `authored/**`;
4. preserve unrelated authored content and working-tree edits;
5. scan for non-authored references in tests, fixtures, docs, examples, qualification contracts, and code;
6. leave non-authored files unchanged and report every known stale reference as engineering follow-up.

Do not leave a known partial authored migration.
