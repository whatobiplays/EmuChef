# BIOS Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** Standalone automated BIOS qualification complete; Phase 6E is In
progress; physical and full end-to-end qualification remain deferred.

## 1. Status and authority

This document records the standalone BIOS-copy qualification originating from
the Phase 6E.2 roadmap sequence. The automated result is limited to the authored
`feature.copy_bios` workflow and does not close Phase 6E or Phase 6D. Phase 6D
remains **In progress** with its deferred evidence requirements unchanged.

## 2. Qualified automated boundary

The strict contract at
`tests/fixtures/recipe-qualification/bios/qualification-contract.json` binds
the raw `authored/recipes/feature.copy_bios.yaml` bytes to SHA-256
`1a3b04aa3f26720701ccbe56336d1f451d3f402c9a092be10ef80682cd9a998b`.
Qualification uses the real authored catalog through
`runtime_configuration::plan_configuration` with `ayaneo.generic.base` as
production capability context and explicitly selects only
`feature.copy_bios`.

The tests exercise production planning, review projection, input validation,
and deterministic execution of the unchanged generated plan. No authored YAML
or device-plan/profile semantics are changed by the qualification.

## 3. What the tests prove

- The required BIOS directory binds with explicit provenance and produces an
  executable, blocker-free production review.
- Missing input and a nonexistent required directory fail before an executable
  plan, digest, or review is produced.
- The authored `copy_files` step retains `sync` policy, the
  `/sdcard/RetroArch/system` destination, and its `path_exists` verification.
- Representative nested BIOS files copy successfully through normal sandbox
  adapters with exact source bytes preserved.
- A private test device wrapper can force the authored destination verification
  to fail after delegated `mkdir_p` and `push` operations, while the generated
  plan remains unchanged.

## 4. What the tests do not prove

This automated qualification does not prove real ADB behavior, device storage
permissions or writability, hardware performance, device cleanup or reset,
packaged-GUI behavior, physical end-to-end success, or combined RetroArch +
BIOS physical behavior. The automated combined result is recorded separately
in [the combined qualification document](recipe-qualification-retroarch-bios.md).

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. Cleanup authority is
`not_authorized_for_recipe_qualification`. No ADB, ignored physical,
manual/operator, live-network, packaged-GUI, signing, notarization, release,
or cleanup qualification ran.

## 6. Combined qualification boundary

Combined RetroArch + BIOS qualification through the real default device-plan
selection is recorded in
[the combined qualification document](recipe-qualification-retroarch-bios.md).
This standalone BIOS result remains limited to the BIOS workflow and must not be
read as the combined qualification itself.
