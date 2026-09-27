---
name: edit-device-profile
description: Edit an existing EmuChef authored device profile under authored/device_profiles, including evidence-backed match/capability changes, connected-device verify/refine flows, and explicitly aligned profile-ID migrations. Use when the user asks to fix or refine device matching, capabilities, Android constraints, metadata, or profile identity. Preserve existing work, avoid speculative match broadening, and update authored references atomically when identity changes.
---

# Edit EmuChef Device Profile

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Resolve the target profile from natural language against the catalog. Ask only when ambiguity remains.
2. Use the working-tree file as the baseline and preserve pre-existing edits.
3. Inspect current typed profile model, matching logic, generator, collision behavior, referencing device plans, and relevant evidence.
4. Apply the same evidence hierarchy as profile creation. Connected observations outrank research; never infer additional Android-reported identifiers from existing YAML or marketing similarity.
5. Support an explicit verify/refine workflow when a physical device becomes available: use the authoritative EmuChef probe/generator path, compare observations with the existing profile, and propose only evidence-backed refinements. Keep exact ADB serials transient.
6. Make the requested change plus directly related coherence fixes only. Preserve textual shape where practical.
7. If the profile ID changes, align and apply one authored migration: identity, conventional filename rename, and every known authored reference under `authored/**`. Report non-authored references without changing them.
8. Align match breadth, capability changes, identity migration, and optional validation before writing.
9. Apply the approved authored edit, inspect the diff, and run only explicitly selected validation.
10. Record evidence and finish with the shared completion report.
