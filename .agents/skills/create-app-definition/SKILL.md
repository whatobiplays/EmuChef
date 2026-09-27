---
name: create-app-definition
description: Create a new EmuChef authored app definition under authored/apps from flexible intent such as an app name, package, APK, repository/release, direct APK URL, or structured metadata. Use when adding a new app/emulator/frontend to the authored catalog. Reuse EmuChef's existing APK/source inspection and generation paths when compatible, research missing facts conservatively, and keep recipe creation delegated to create-recipe. Hand off confirmed duplicates to edit-app-definition.
---

# Create EmuChef App Definition

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Evidence authority

Prefer evidence in this order when facts conflict:

1. APK inspection;
2. primary upstream repository/release metadata;
3. official project documentation;
4. other reliable sources;
5. explicit user-supplied but independently unverified facts.

User decisions may override presentation/authoring choices, but never relabel contrary technical evidence as verified.

## Workflow

1. Accept flexible starting information: app identity, package, APK, supported remote source, direct APK URL, or structured facts.
2. Inspect the current app-definition model, APK inspection/generation paths, remote-source generation logic, existing app definitions, and related recipes/plans.
3. When the input is compatible with an existing EmuChef inspection/generator path, use that path as the primary proposal authority. If it also proposes a recipe, keep only the app proposal here; recipe creation remains owned by `create-recipe` unless the aligned multi-skill change explicitly includes it.
4. When the existing generator cannot handle the source but reliable evidence exists, create from research. Leave unverifiable values conservative/empty according to the current schema rather than inventing them.
5. Search for a logical duplicate before writing. If one exists, explain the match and ask whether to switch to `edit-app-definition`.
6. Align on identity, package/source semantics, user-facing metadata, provisioning/config-path facts, artifact expectations, and any companion authored objects.
7. Write only the new app definition under `authored/apps/**` from this skill.
8. Apply optional static validation selected during alignment.
9. Record evidence and finish with the shared completion report.
