# XaniteOG Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** Standalone automated XaniteOG qualification complete; Phase 6E is
In progress; physical and full end-to-end qualification remain deferred.

## 1. Status and authority

This document records the standalone automated qualification for the authored
`app.xaniteog.install` workflow. The result advances bounded automated Phase 6E
coverage without closing Phase 6E or Phase 6D. Phase 6D remains **In progress**
with its existing missing-evidence requirements unchanged.

The production planning context is `ayaneo.pocket_s2.base`, which supplies the
capability profile required by the authored APK-install step. Qualification
explicitly selects only `app.xaniteog.install`; the plan's existing XaniteOG
membership and default selection are not treated as product provenance.

## 2. Qualified automated boundary

The strict contract at
`tests/fixtures/recipe-qualification/xaniteog/qualification-contract.json`
binds the raw `authored/recipes/app.xaniteog.install.yaml` bytes to SHA-256
`c55f9547f7aa8de60f951386243992fa9b8c66329eeb6e1fd2f20bea21dda1f2`.
The active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_xaniteog_tests.rs`.

Qualification uses the real authored catalog through
`CatalogSnapshot::legacy_local` and production
`runtime_configuration::plan_configuration`. A valid local `.apk` file is
bound explicitly to the authored required input. The generated plan is
executed unchanged through deterministic sandbox-root adapters.

## 3. What the tests prove

- The strict contract matches recipe identity, source hash, required APK input
  validation, `apk_install`, the `install_apk` operation, `Ali.Xanite`, the
  authored input reference, `replace_existing: false`, and the
  `package_installed` skip predicate.
- Production planning explicitly selects only `app.xaniteog.install`, emits a
  blocker-free executable review, and presents the configured APK as its
  filename without exposing its host parent directory or runtime authority
  details.
- Missing, nonexistent, wrong-extension, and wrong-path-kind APK bindings fail
  closed before executable plan, digest, or review authority is produced.
- The unchanged generated plan installs successfully through deterministic
  sandbox adapters without live network access or ADB.
- A repeated deterministic run remains successful while skipping installation
  after the `Ali.Xanite` package state is recorded.
- A private test-only device adapter can force installation failure while the
  generated plan remains unchanged and the failed result makes no false later
  success claim.

## 4. What the tests do not prove

This automated qualification does not prove that the seeded fixture bytes are a
valid Android APK, Android package-manager behavior, real ADB behavior, device
permissions or storage writability, hardware compatibility, XaniteOG launch
behavior, device cleanup or reset, packaged-GUI behavior, release readiness, or
physical end-to-end success. It does not qualify another recipe or derive
XaniteOG provenance from device-plan membership.

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. Cleanup authority is
`not_authorized_for_recipe_qualification`. No ADB, ignored physical,
manual/operator, live-network, packaged-GUI, signing, release, or cleanup
qualification ran as part of this automated workflow. Ordinary production real
execution remains behind its existing gating boundary.

## 6. Roadmap boundary

This result adds standalone automated coverage for the authored XaniteOG
workflow. RetroArch, BIOS, Obtainium, ROM/content-copy, and the combined
RetroArch + BIOS qualification remain separately documented. Phase 6D and Phase
6E remain **In progress**; remaining authored workflows and physical/full
end-to-end qualification remain subject to the existing roadmap sequencing.
