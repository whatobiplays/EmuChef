---
name: diagnose-authored-object
description: Diagnose failures or unexpected behavior caused by EmuChef authored catalog data across recipes, device profiles, app definitions, and device plans. Use for symptoms such as planning failures, profile mismatches, unavailable setups, validation errors, execution failures, broken references, or questions like why a recipe/plan/device setup does not work. Trace relevant authored relationships and authoritative runtime/planner semantics; if repair is authored-only, align it and delegate to the matching edit skill. Never change EmuChef code.
---

# Diagnose EmuChef Authored Object

Before doing any work, load and obey `../authored-object/SKILL.md` and its referenced base policies.

## Workflow

1. Start from the user's symptom. Accept natural language, object IDs/names, file paths, validation output, execution failures, or observed product behavior; do not require the user to identify the responsible YAML.
2. Inspect the relevant authored dependency/reference graph across `authored/**` and the authoritative runtime/planner/generator semantics needed to explain the symptom.
3. Use existing qualification/evidence and external primary sources when they materially help establish a technical fact.
4. Keep the investigation relevance-bounded. Follow causal references; do not indiscriminately review unrelated code or authored objects.
5. Do not modify authored data during diagnosis.
6. Do not run repository validation commands or physical execution without explicit approval, even when the command is read-only. File/repository inspection and reasoning do not require validation approval.
7. Establish a concrete root cause and classify it:
   - **authored-data defect**: present the root cause and proposed semantic repair for alignment, then invoke the appropriate specialized edit skill or skills;
   - **environment/input issue**: identify the external condition and what would permit a retry; do not edit YAML merely to tolerate the environment;
   - **runtime/product limitation or bug**: identify the exact unsupported or misbehaving boundary and return it to the normal engineering workflow. Do not modify code or invent a semantically inferior authored workaround.
8. When repair is aligned, let the specialized editor own the mutation and its validation choices.
9. Retain diagnosis evidence locally and use the shared completion report, including unresolved non-authored follow-up.

Complete diagnosis only when the causal boundary is specific enough that the next action is clear and no unsupported inference is presented as fact.
