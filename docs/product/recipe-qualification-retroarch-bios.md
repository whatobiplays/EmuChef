# RetroArch and BIOS Plan Membership Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** Host tests confirm default-plan membership and BIOS-only copy
behavior for `ayaneo.konkr_pocket_fit.base`. Combined provisioning and BIOS
copy review is not prepared because provisioning includes runtime-generated
copy sources. Physical, manual, and full end-to-end qualification remain
deferred.

## 1. Status and authority

This document records host evidence for the AYANEO KONKR Pocket Fit plan. It
does not qualify the separate KONKR Pocket Advance profile or plan. The
qualification uses
`selected_recipes: None`, so the selected recipe set comes from the real
device-plan defaults rather than from a test-specific recipe list. The authored
default order is:

1. `app.retroarch.provision`
2. `feature.copy_bios`

The qualification uses production catalog loading, planning, review projection,
and executor adapters. The source-bound contract below describes the current
schema-v1 Recipes and App Definition after App Artifact and named-target
migration.

## 2. Source-bound contract

The strict contract is
`tests/fixtures/recipe-qualification/retroarch-bios/qualification-contract.json`.
It binds the raw authored sources as follows:

The RetroArch App Definition owns the `apk` artifact and the seven frontend,
thirteen core, and three system-resource archives as named App Artifacts. The
Recipe resolves these through App Artifact references and retains cache
selection, artifact groups, extraction, and copy behavior. The BIOS Recipe and
its input-owned content remain unchanged.

| Source | SHA-256 |
|---|---|
| `authored/recipes/app.retroarch.provision.yaml` | `8f626cf663790286e0e15721918d713cb1e05ad39bad8976c3d0d3af35b46b7a` |
| `authored/apps/retroarch.yaml` | `eb322b675ef8858b833c9b8605dd4a7a371c073c31672d842e6bd1e8a058fa05` |
| `authored/recipes/feature.copy_bios.yaml` | `4f4471aacb0cacebffcebb4785b12f0e7ef43d306526031540d0496957840fbc` |
| `authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml` | `3da268bf1ce2dca8600baa47d7663a5d6542c0b5cd8c9a41510f2e9d71a3a969` |
| `authored/device_profiles/ayaneo.konkr_pocket_fit.yaml` | `c1cef00eacf96760f1e21f405621b08e8b6070b983cfa5ec064209caf61a2db9` |

The required external input is
`feature.copy_bios/bios_source_dir`; `app.retroarch.provision/retroarch_cfg`
remains optional. Fake `/sdcard` and `/storage/emulated/0` equivalence is not
qualified by the fake filesystem.
The App Definition source digest identifies catalog declarations; it is not
the SHA-256 digest of the APK selected for a device installation.

## 3. Production planning and review

The qualification proves that `CatalogSnapshot::legacy_local` and
`runtime_configuration::plan_configuration` produce a typed plan containing
exactly the two default workflows when the required BIOS directory is valid.
Without that binding, production planning returns `binding_missing` and
produces no plan.

The combined plan requires CopyCommit snapshot preparation before it has a
review digest or executable review. Preparation fails closed because the
RetroArch provisioning Recipe contains App Target copy sources produced by
earlier runtime steps. Their bytes cannot be captured before execution, so the
combined provisioning-plus-BIOS flow is not execution-ready and is not
qualified by these tests.

## 4. Deterministic execution behavior

The selected BIOS-only plan succeeds through normal sandbox adapters using
seeded default-cache fixtures and representative nested BIOS files. Copied
BIOS bytes are verified exactly. This test does not execute RetroArch
provisioning.

A repeated BIOS-only run freshly reviews the destination and executes the
authored `sync` copy again. A forced BIOS destination verification failure
keeps prior results truthful and fails the BIOS-only run.

The planner accepts independent Recipe operations with overlapping App
Targets. That membership evidence does not establish that dynamically generated
copy sources can be reviewed or executed as one combined plan.

## 5. Qualification boundary

This automated result does not prove ADB behavior, Android storage alias
semantics, device permissions or writability, hardware performance, physical
cleanup/reset, packaged-GUI behavior, operator/manual execution, or physical
full end-to-end success. Physical cleanup authority remains
`not_authorized_for_recipe_qualification`.

Phase 6D remains **In progress** with its missing physical and UI-smoke
evidence requirements unchanged. Phase 6E remains **In progress** because
additional authored workflows and physical/full end-to-end qualification remain
open. Ordinary production real execution remains behind its existing gating
boundary.

The separate operation-evidence requirements for the KONKR Pocket Advance are
recorded in the [qualification matrix](../../tests/fixtures/recipe-qualification/konkr-pocket-advance/operation-evidence-matrix.json).
It records required evidence fields only; no physical measurements or
execution results are present.

The standalone results are documented in the
[RetroArch](recipe-qualification-retroarch.md) and
[BIOS](recipe-qualification-bios.md) qualification documents.
