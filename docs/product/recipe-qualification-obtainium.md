# Obtainium Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Standalone automated Obtainium qualification complete; Phase 6E is
In progress; physical and full end-to-end qualification remain deferred.

## 1. Status and authority

This document records the standalone automated qualification for the authored
`app.obtainium.install` workflow. It is the next independent workflow in the
Phase 6E initial qualification set after RetroArch. The result does not close
Phase 6E or Phase 6D. Phase 6D remains **In progress** with its existing
missing-evidence requirements unchanged.

The production planning context is `ayaneo.generic.base`, which supplies the
capability profile needed by the authored install step. The qualification
explicitly selects only `app.obtainium.install`; it does not add Obtainium to
that device plan and does not use device-plan membership as product
provenance.

## 2. Qualified automated boundary

The strict contract at
`tests/fixtures/recipe-qualification/obtainium/qualification-contract.json`
binds the raw
`authored/recipes/app.obtainium.install.yaml` bytes to SHA-256
`d3f96f4d6f0fa812af75b0ddc18edad9da69b7b2ceae62468c0bd3c8b645caa7`.
The active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_obtainium_tests.rs`.

Qualification uses the real authored catalog through
`CatalogSnapshot::legacy_local` and production
`runtime_configuration::plan_configuration`. It retains the authored
GitHub URL and `cache: default` semantics. The deterministic executor tests
seed the exact production-derived default-cache filename and execute the
unchanged generated plan through
`ExecutorAdapters::with_sandbox_roots`.

## 3. What the tests prove

- The authored source, explicit recipe selection, expanded recipe set,
  `apk_install` capability, operation families, dependency edge, artifact,
  package identity, and `package_installed` skip predicate match the strict
  qualification contract.
- Production planning uses `ayaneo.generic.base` only as capability context,
  produces no resolved inputs, and emits a blocker-free production review for
  one explicit Obtainium feature.
- The generated plan resolves the seeded artifact and installs successfully
  through deterministic sandbox adapters without live network access or ADB.
- A repeated deterministic run skips the install from package state produced by
  the first run while remaining successful.
- A private test-only device adapter can force the install operation to fail
  while the unchanged generated plan, completed artifact resolution, failed
  install record, and absence of later successful execution remain truthful.

## 4. What the tests do not prove

This automated qualification does not prove GitHub availability, acceptance of
the seeded bytes as a valid Android APK, Android package-manager behavior,
permissions or app-ops, device storage behavior, hardware compatibility,
actual Obtainium launch behavior, device cleanup or reset, packaged-GUI
behavior, or physical end-to-end success. It also does not qualify any other
recipe or establish Obtainium provenance for a device plan.

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. Cleanup authority is
`not_authorized_for_recipe_qualification`. No ADB, ignored physical, manual or
operator, live-network, packaged-GUI, signing, release, or cleanup
qualification ran as part of this automated workflow. Ordinary production real
execution remains disabled behind its existing gating boundary.

## 6. Roadmap boundary

RetroArch and BIOS are qualified separately, and the real default combined
RetroArch + BIOS workflow is recorded in
[the combined qualification document](recipe-qualification-retroarch-bios.md).
The next remaining item in the initial Phase 6E set is ROM/content copying;
first-launch and later frontend workflows remain separate roadmap work.
