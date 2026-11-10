# T08: Typed Template Variables and Standalone Configuration Rendering

**Status:** Approved design (2026-11-09); implementation and qualification pending.
**Scope:** EmuChef Issue #51, the T08 extension to the completed T07 authored App Definition and Recipe contracts.
**Authority:** ADR 0006, `CONTEXT.md` (app-owned configuration contracts), and the six product-owner-approved decisions of November 9, 2026.

## Purpose, terminology, and invariants

A **Template Variable** is a declared typed value substituted into one Configuration Template. A **Managed Property** is an App Definition-owned setting with a canonical property path, type, constraints, optional default and clearability. They are different domain concepts. A variable must never bypass validation of an app-owned managed property it happens to populate. A Recipe may expose an existing managed property but cannot create new managed-property editing authority.

T08 creates deterministic reviewable bytes, not permission to deploy. Retain T07 compatibility, T03 drift protections, current Setup Location identity guards, and the separate T09 existing-file merge and T13a publication boundaries. As previously decided for T05, independent uncooperative external-writer syscall races are excluded from the Setup Location threat model, while EmuChef's stale reviews, session/identity changes, and internal concurrent mutations are guarded.

## 1. Declaration ownership and types

A catalog App Configuration Template owns its variable declarations under the owning App Definition; a user-supplied template may carry separately validated declaration metadata admitted by the trusted runtime. IDs are unique **per template**, independent of managed-property IDs. Define one bounded, unambiguous identifier grammar and validate every declaration and reference.

Supported variable types: `string`, `integer`, `number`, `boolean`, and template-only `device_path`. The first four retain existing T07 scalar semantics and applicable validation. The fifth is an explicitly resolved Android-device path, not a new general-purpose `ConfigValueTypeV1` managed-property kind. Do not coerce strings into numbers, booleans or paths or silently convert a host `path` to `device_path`.

Reuse existing validators at their correct authority level; app-owned managed-property validators still apply to any final value at that managed path. If an existing Recipe input type cannot supply an approved variable type, especially nonintegral `number`, make only the narrow ordinary input-declaration support needed for typed input binding, with unchanged input precedence and no parallel config-value source. Expose any real compatibility obstruction before broadening that contract.

## 2. Value bindings

Recipe `config.variables` explicitly maps each used declared variable ID to a Recipe input ID. Bind through the *existing resolved input* model: input defaults, requiredness, validation, effective user values, source precedence and `sensitive` carry through. There is no separate template-variable default or precedence system. Referencing a variable whose effective value is missing always fails; a declared optional variable that is not referenced need not be bound.

A template `device_path` can bind a validated `device_path` input or a current trusted `setup_location_ref` through the T05 resolver. Permit `setup_location_ref` in this **T08-only** configuration-variable position without relaxing T07's `copy_files.dest` restriction for any other operation. Reject incompatible input types, multiple-value shapes, undeclared variable IDs and unsafe host/device-path confusion.

## 3. Grammar and encoding

The v1 placeholder is `{{emuchef:<variable-id>}}`. Repeated references are allowed; every reference must occupy an **entire scalar value**. No substring interpolation, property/key-name placeholders, comments, loops, conditionals, computed expressions, or arbitrary textual replacement.

For `json/v1`, parse the document before substitution. A placeholder is recognized only as the complete value of a string-scalar node; replace that node with the correctly typed JSON scalar. Preserve numeric and boolean JSON types and use safe deterministic serialization/escaping.

For `key_value/v1`, parse admitted key/value statements first, allow placeholders only in complete value positions, and use format-aware scalar encoding. Refuse any value that cannot be represented unambiguously, including newline, comment or delimiter injection. Never do an unchecked whole-file search/replace.

Retain bounded, safely parsed unknown *literal* user-template properties as opaque unmanaged data. They grant neither unknown managed overrides nor executable expansion. Reject malformed/oversized templates, unsupported encodings, unknown or partial tokens, ambiguous duplicate keys and unsafe expansion. Existing T07 per-configuration input limits remain applicable.

## 4. Missing, duplicate and invalid inputs

Duplicate variable declarations and duplicate binding IDs are errors; repeated valid *references* are permitted. An unresolved referenced variable (even when its input was optional), an undeclared reference, a missing required input, a type/validator failure or an unsupported encoding stops rendering with a sanitized diagnostic. No default guessing, silent deletion, coercion, or empty-string substitution.

An explicit empty value is not missing. Retain T07's distinct absent, explicit value, `clear` and `reset` semantics. A surplus or undeclared binding cannot grant new authority.

## 5. Sensitivity, paths and review identity

Source `InputDeclaration.sensitive` propagates to the variable and cannot be downgraded. Neither raw sensitive values nor per-variable value-derived digests may appear in diagnostics, provenance, logs, or ordinary presentation. Actual rendered bytes may legitimately contain sensitive configuration values and must be held within protected trusted artifact handling. The mandatory SHA-256 of the *whole rendered artifact* is content identity for the trusted review path, not permission to expose sensitive inputs or their fingerprints.

Store reusable templates and binding intent with logical/portable path references. Resolve `setup_location_ref` only against the current trusted device/session/Setup Location identity; the *frozen, destination-bound rendered bytes* may include the concrete Android device path because that is what the app consumes. Bind review to exact bytes/hash, format, logical destination and current authority; invalidate/re-render for changed relevant inputs, catalog authority, session, device or location. Do not persist a stale physical mount path as reusable intent.

## 6. Managed properties, precedence and reset

Use the ADR 0006 pipeline: select and validate base/format/target; resolve typed input values and device paths; parse and substitute whole-value tokens; validate substituted managed-property paths against the owning App Definition; apply declared direct managed-property operations; validate the resulting configuration; deterministically serialize exact bytes with SHA-256 and versioned sanitized provenance.

If substitution and an explicit managed-property edit affect the same field, **the explicit managed-property operation wins**; record the collision/winner in trusted non-sensitive provenance. Without a direct edit, a variable populating a managed property remains subject to its validators. `unmanaged_literals` remain static opaque data, not a substitute for declared variables or an avenue for unknown managed edits.

`reset: true` means T11 recommendation recomputation. T08 **refuses unresolved reset**. It may consume an already trusted, valid, pre-resolved effective value plus recommendation-source provenance when available, but must never select a recommendation itself or substitute `clear` or the App Definition default. `clear` applies only to clearable managed properties. Explicit empty and absent remain distinct.

## Verification and exclusions

Test exact repeated bytes/digests and provenance, two genuinely different format fixtures (`key_value/v1`, `json/v1`), typed scalar and path values, repeated references, safe escaping, unknown literal preservation, invalid/duplicate/missing values, managed-property collisions, sensitivity redaction, stale device authority and unresolved reset. A second format fixture is not qualification of another emulator or physical device.

Outside T08: T09 existing-file merging and multi-Recipe composition; T13a create-only device publication; T11 recommendation calculation; Config Editor feature work; OEM privilege execution and physical device qualification.

## Rationale and rejected alternatives

1. Keep template variables distinct from managed properties: settings must remain app-owned, while templates also need non-setting inputs such as device paths. Rejected: using managed properties as the sole substitution schema.
2. Explicit Recipe input binding reuses defaults, validation, sensitivity and precedence. Rejected: independent config binding values, silent inference or secondary precedence.
3. Whole-scalar placeholders enable format-aware typed parsing and encoding. Rejected for v1: substring interpolation and a general template language.
4. Fail-closed missing and duplicate rules prevent silent configuration corruption. Rejected: coercion, empty fallback and automatic omission of referenced tokens.
5. Current-session path resolution and inherited sensitivity prevent stale-device and diagnostic leakage. Rejected: persisting reusable device-physical paths or publicly fingerprinting sensitive values.
6. Direct managed edits win; unresolved reset blocks. Rejected: treating reset as clear/default or computing T11 recommendations inside T08.

**Implementation is not yet complete.** This approval is a design contract, not evidence of rendering, review, device execution or release readiness.
