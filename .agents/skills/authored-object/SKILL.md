---
name: authored-object
description: Shared policy and orchestration for EmuChef repo-local authored catalog work across recipes, device profiles, app definitions, and device plans. Specialized create/edit/diagnose skills must load this first. Use directly for generic or multi-object authored-catalog requests, routing, collision handling, evidence collection, ID migrations, validation choices, or coordinated authored-only changes that span object types.
---

# EmuChef Authored Object

Use this as the shared control plane for every EmuChef authored-catalog skill. Keep object-specific semantics in the specialized skill; keep cross-cutting policy here.

## Core invariant

Operate on authored data, not product code.

Normal mutation ownership:

- recipes: `authored/recipes/**`
- device profiles: `authored/device_profiles/**`
- app definitions: `authored/apps/**`
- device plans: `authored/device_plans/**`
- local evidence: `.chatgpt/authoring-evidence/**`

An explicitly aligned ID migration may update any referencing file under `authored/**`. Treat that as a narrow reference-driven exception, not general write authority.

Keep source code, tests, fixtures, docs, `CONTEXT.md`, build files, lockfiles, and other non-authored production files unchanged. Report required non-authored follow-up instead of changing those files.

Leave Git lifecycle alone: do not stage, commit, push, stash, reset, clean, or discard user changes.

## Start every workflow

1. Load root `AGENTS.md` and the repository guidance needed for the task.
2. Inspect the working tree read-only. Distinguish pre-existing changes from this workflow's changes.
3. If the target authored file is already modified, use the current working-tree version as the baseline. Preserve those edits and surface semantic conflicts rather than replacing them from HEAD.
4. Inspect the current authoritative EmuChef implementation for the object being handled: typed models, generators, planner/runtime semantics, existing authored examples, reference relationships, and current validation tooling. Do not treat this skill as a schema cache.
5. Search the authored catalog before proposing a new object. Prefer reuse/composition over duplicated catalog behavior.

Read [ownership-and-composition.md](references/ownership-and-composition.md) for routing, collisions, transactions, and migrations.

## Evidence before decisions

Collect facts yourself when the repository, connected device, APK/source inspection, or reliable external sources can answer them. Ask the user for product decisions, not discoverable facts.

Use external research only when it resolves a material missing fact, verifies potentially stale information, or materially improves confidence. Prefer primary upstream evidence. If high-quality sources materially conflict, surface the conflict during alignment rather than choosing silently.

Classify important facts as observed/verified, derived, suggested, user-supplied, missing, or conflicting. A user-supplied technical fact is usable evidence but is not independently verified merely because the user supplied it.

Read [evidence.md](references/evidence.md) before persisting authoring evidence.

## Align before mutation

Before writing, present the smallest coherent authored change set and get alignment on every material behavioral choice. Include:

- authored objects/files to create or edit;
- intended semantics and meaningful alternatives;
- identity changes or cross-object reference updates;
- assumptions or unresolved evidence conflicts;
- optional validation choices;
- for optional physical validation, the chosen target, expected mutations, required user-owned inputs, and expected postconditions.

Do not re-ask decisions already explicit in the request.

If a later branch discovers a new material ambiguity, stop that branch and return only that new decision for alignment. Continue independent approved work only when doing so cannot leave the catalog incoherent.

## Apply one logical authored change

For a composed request, align the complete required authored-object set first. Then apply the approved set as one logical catalog operation. Do not knowingly leave half of a required cross-skill change applied.

A specialized skill owns its object type. When another object must be created or semantically edited, invoke the corresponding specialized skill rather than absorbing that object's semantics into the current skill.

After writing, inspect the resulting diff against the approved semantics. Even when repository validation is declined, perform this bounded authoring review and report the result as not repository-validated.

## Validation is optional

Never run repository validation commands or physical execution without explicit approval. Offer relevant validation during alignment unless the user's request already selected or declined it.

When validation is approved, follow [validation.md](references/validation.md). Production execution may still perform its mandatory admission/planning/preflight checks even when the separate static-validation option was declined.

## Record and report

Retain a local ignored evidence note for each completed authored workflow. Use [reporting.md](references/reporting.md) for the evidence record and final result shape.

Complete only when every approved authored object is accounted for, the resulting diff matches the aligned semantics, evidence is recorded, and validation/follow-up status is explicit.
