---
name: edit-app-definition
description: Edit an existing EmuChef authored app definition under authored/apps using current APK/source inspection, upstream evidence, and catalog semantics. Use when the user asks to update, fix, refine, rename, migrate, or refresh an app definition, package/source metadata, provisioning paths, or app identity. Preserve current working-tree edits, distinguish user choices from verified facts, and update authored references atomically for explicitly aligned ID migrations.
---

# Edit EmuChef App Definition

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Resolve the target app from natural language against the current catalog; ask only for genuine ambiguity.
2. Use the current working-tree version as the baseline and preserve pre-existing edits.
3. Inspect the current app model, relevant APK/source inspection and generator paths, related recipes/plans, and existing evidence.
4. Use the same evidence authority as creation: APK inspection, primary upstream repository/release metadata, official documentation, other reliable sources, then explicit unverified user facts. Existing YAML does not outrank stronger new technical evidence merely because it is old.
5. User decisions may control authoring/presentation choices but do not relabel contrary technical evidence as verified.
6. Make the requested semantic change plus directly related coherence fixes only; preserve textual shape where practical.
7. If the app ID changes, align and apply one authored migration: identity, conventional filename rename, and every known authored reference under `authored/**`. Report non-authored references without changing them.
8. Align material changes, evidence conflicts, migration scope, and optional validation before writing.
9. Apply the approved authored edit, inspect the diff, and run only explicitly selected validation.
10. Record evidence and finish with the shared completion report.
