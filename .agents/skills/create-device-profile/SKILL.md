---
name: create-device-profile
description: Create a new EmuChef authored device profile under authored/device_profiles using either a connected Android device or trustworthy supplied/researched facts. Use when adding support for a handheld/device, capturing device matching criteria, establishing capability defaults, or creating a profile for a model not yet represented. Prefer EmuChef's existing connected-device generator and conservative matching rules; never invent identifiers or broaden matching speculatively. Hand off confirmed duplicates to edit-device-profile.
---

# Create EmuChef Device Profile

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Choose evidence mode

Use one explicit mode:

### Connected mode

Prefer this mode when an appropriate device is available.

1. Use EmuChef's existing device listing/probe/device-profile-generation path as the primary proposal authority.
2. Keep exact ADB serials and other transport authority transient. Do not persist them in YAML, evidence notes, or product-facing output.
3. Treat the generator's evidence classifications and conservative defaults as the baseline. Align only on meaningful edits to that proposal.
4. Do not independently reinterpret raw ADB output when the current generator already owns the same semantic translation.

### Research mode

Use trustworthy user-supplied or external evidence when no device is available.

1. Verify manufacturer/model/Android identifiers from primary or otherwise reliable sources where possible.
2. Include only reported identifiers supported by evidence. Do not infer marketing aliases as Android-reported strings or write broad family regexes merely because they seem plausible.
3. Follow the current generator's conservative capability proposal semantics. Unknown or higher-risk capabilities stay conservative; distinguish suggested capability values from verified ones during alignment.
4. Do not invent missing match criteria merely to make the profile broad enough to seem useful.

## Workflow

1. Inspect current typed device-profile models, matching logic, generator, collision logic, existing profiles, and any referenced device plans.
2. Search for an existing logical profile before creating a new one. If it already exists, explain the match and ask whether to switch to `edit-device-profile`.
3. Build the narrowest evidence-backed match criteria and capability defaults consistent with current EmuChef semantics.
4. Align on matching breadth, capability proposals, identity/name, Android constraints, tags/metadata, and any required companion authored objects.
5. Write only the new profile under `authored/device_profiles/**` from this skill.
6. Apply optional static validation selected during alignment.
7. Record evidence and finish with the shared completion report.

Connected probing is evidence collection for this workflow, not implicit approval to run unrelated validation or mutate the device.
