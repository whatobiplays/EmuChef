# Combined RetroArch + BIOS Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** Automated qualification complete for the real
`ayaneo.konkr_pocket_fit.base` default; Phase 6E is In progress; physical,
manual, and full end-to-end qualification remain deferred.

## 1. Status and authority

This document records the composition-level automated qualification of the
authored KONKR default device plan. The qualification uses
`selected_recipes: None`, so the selected recipe set comes from the real
device-plan defaults rather than from a test-specific recipe list. The authored
default order is:

1. `app.retroarch.provision`
2. `feature.copy_bios`

The qualification uses production catalog loading, planning, review projection,
and executor adapters. The source-bound contract below describes the current
authored Recipes and App Definition after named target migration.

## 2. Source-bound contract

The strict contract is
`tests/fixtures/recipe-qualification/retroarch-bios/qualification-contract.json`.
It binds the raw authored sources as follows:

The RetroArch App Definition supplies the seven frontend, thirteen core, and
three system-resource archives as named `kind: file` artifacts. Its Recipe
keeps the APK `remote_file`, cache selection, artifact groups, extraction, and
copy behavior; the BIOS Recipe and its input-owned content remain unchanged.

| Source | SHA-256 |
|---|---|
| `authored/recipes/app.retroarch.provision.yaml` | `bf10874d72be73dc8a5255101403bbe025f6e073f1891a7146a9aa812ed173a2` |
| `authored/apps/retroarch.yaml` | `795e26f79d6f1eb2edccd76a0774a7ce5dbaa18fdcb13bad0572734640aac263` |
| `authored/recipes/feature.copy_bios.yaml` | `4f4471aacb0cacebffcebb4785b12f0e7ef43d306526031540d0496957840fbc` |
| `authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml` | `3da268bf1ce2dca8600baa47d7663a5d6542c0b5cd8c9a41510f2e9d71a3a969` |
| `authored/device_profiles/ayaneo.konkr_pocket_fit.yaml` | `c1cef00eacf96760f1e21f405621b08e8b6070b983cfa5ec064209caf61a2db9` |

The required external input is
`feature.copy_bios/bios_source_dir`; `app.retroarch.provision/retroarch_cfg`
remains optional. Fake `/sdcard` and `/storage/emulated/0` equivalence is not
qualified by the fake filesystem.

## 3. Production planning and review

The qualification proves that `CatalogSnapshot::legacy_local` and
`runtime_configuration::plan_configuration` produce exactly the two default
workflows when the required BIOS directory is valid. Without that binding,
production planning returns `binding_missing` and produces no executable plan,
digest, or review.

The valid production review is executable and blocker-free, contains exactly
two features and both recipe workflows, has an action count matching the
generated plan, and does not expose temporary host parent paths or device
authority.

## 4. Deterministic execution behavior

The exact production-generated combined plan succeeds through normal sandbox
adapters using seeded RetroArch default-cache fixtures and representative
nested BIOS files. Copied BIOS bytes are verified exactly.

A second execution of the same plan on the same deterministic device state
skips the authored RetroArch install through `package_installed`, still runs
the BIOS sync copy, and remains successful.

A test-private device wrapper can force only the authored BIOS destination
verification for `/storage/emulated/0/RetroArch/system` to return false. The unchanged plan
is executed without adding cross-recipe dependencies or controlling step
ordering: BIOS copy operations occur before that verification failure, prior
results remain truthful, the BIOS step and overall run fail, and no later work
is fabricated as successful.

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

The standalone results are documented in the
[RetroArch](recipe-qualification-retroarch.md) and
[BIOS](recipe-qualification-bios.md) qualification documents.
