# ROM Library Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Standalone automated ROM-copy qualification complete; Phase 6E is In
progress; physical and full end-to-end qualification remain deferred.

## 1. Status and authority

This document records the standalone qualification of the authored
`feature.copy_roms` workflow. The result advances automated Phase 6E coverage
without closing Phase 6E or Phase 6D. Phase 6D remains **In progress** with its
deferred physical and UI evidence requirements unchanged.

## 2. Qualified automated boundary

The strict contract at
`tests/fixtures/recipe-qualification/roms/qualification-contract.json` binds
the raw `authored/recipes/feature.copy_roms.yaml` bytes to SHA-256
`956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041`.
Qualification uses the real authored catalog through
`runtime_configuration::plan_configuration` with `ayaneo.generic.base` as
production capability context and explicitly selects only
`feature.copy_roms`.

The generated `copy_files` plan is executed unchanged through deterministic
sandbox-root adapters. The qualification covers the authored default
destination `/sdcard/ROMs`, the allowed `/storage/emulated/0` destination
prefix, and the authored `merge`, `replace`, and `sync` policies.

## 3. What the tests prove

- The required existing source directory, default destination, default `merge`
  policy, accepted alternate destination, and all authored policy options plan
  through production configuration and review.
- Missing and nonexistent source directories, disallowed destinations, and an
  unsupported policy fail closed before an executable plan, digest, or review
  is produced.
- The production review is executable and blocker-free, exposes one copy
  action, summarizes the source without its full host parent path, and does not
  expose a serial or runtime authority.
- Default `merge` preserves unrelated destination content, replaces colliding
  source files with exact bytes, preserves nested layout, and leaves the
  generated `ExecutionPlan` unchanged.
- `replace` clears stale destination content before copying the nested source.
- Directory-style `sync` mirrors its source and removes destination-only files.
  This behavior is the authored “Mirror source” contract. The correction was
  limited to the shared fake-device directory-copy branch proven defective by
  the production-generated qualification and its lower-level executor
  regression; single-file, path-list, and unrelated execution branches were
  not independently redesigned.
- A deterministic device-operation failure produces a failed copy record while
  preserving the generated plan and making no verification or downstream
  execution claim. The authored recipe has `verify: []`, so no verification
  predicate coverage is claimed.

## 4. What the tests do not prove

This automated qualification does not prove real ADB behavior, device storage
permissions or writability, hardware performance, device cleanup or reset,
packaged-GUI behavior, physical end-to-end success, release readiness, or
physical behavior across device classes.

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. Cleanup authority is
`not_authorized_for_recipe_qualification`. No ADB, ignored physical,
manual/operator, live-network, packaged-GUI, signing, notarization, release,
or cleanup qualification ran.
