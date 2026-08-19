# RetroArch Recipe Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** Automated qualification complete for
`app.retroarch.provision`; Phase 6E is In progress; physical and full
end-to-end qualification remain deferred.

## 1. Status and authority

This document records the standalone RetroArch automated qualification that
originated as Phase 6E.1 roadmap work. The owner explicitly deferred all
remaining manual and physical qualification without closing Phase 6D. Phase
6D therefore remains **In progress** with every existing missing-evidence and
closure requirement unchanged; the deferral is a sequencing decision, not a
waiver. Ordinary production real execution remains disabled behind its
existing gating boundary.

## 2. Qualified automated boundary

The qualification covers the real authored
`app.retroarch.provision` workflow through production code paths only:

- a strict qualification contract at
  `tests/fixtures/recipe-qualification/retroarch/qualification-contract.json` bound by
  SHA-256 to the raw authored recipe bytes, so authored changes fail closed
  until expectations are deliberately reviewed;
- real catalog admission through `CatalogSnapshot::legacy_local` and production
  planning through `runtime_configuration::plan_configuration` with the
  `ayaneo.konkr_pocket_fit.base` device plan;
- the production review projection returned by planning;
- deterministic execution of the unchanged generated plan through
  `ExecutorAdapters::with_sandbox_roots` with pre-seeded default-cache
  fixtures, so no live public network access is required.

## 3. What the tests prove

- Recipe admission, expansion, capability context, optional/supplied config
  behavior, and material dependency ordering match the qualification contract.
- The production review remains executable and blocker-free, covers the
  expected action sections and the authored 7-second aggregate wait total,
  and sanitizes file-input summaries without leaking host parent paths.
- The source-bound contract and generated plan preserve both authored
  first-launch sequences: bootstrap launch, 1500 ms wait, and force-stop;
  then post-permission launch, 5000 ms wait, and force-stop, with direct
  dependency and ordering assertions.
- Artifact definitions enter the production resolution model at their exact
  default-cache filenames with authored URLs unchanged.
- The generated workflow completes successfully through the deterministic
  dry-run adapters with no Failed, Blocked, or Cancelled record, including
  successful execution records for all six first-launch lifecycle steps.
- A test-private bootstrap force-stop failure preserves completed installation,
  launch, and wait results, reports the lifecycle failure, and blocks dependent
  permission and downstream work without mutating the generated plan.
- A repeated deterministic run skips the authored `install_retroarch` step
  through the production `package_installed` predicate while remaining
  successful.
- Omitting a required core-system directory fails the authored
  `copy_core_system_files` verification, retains prior completed results, and
  prevents the final launch from being reported as successfully executed.

## 4. What the tests do not prove

The deterministic qualification does not prove real download service availability,
APK acceptance on Android, Android permission/app-op behavior, private or
shared-storage semantics on hardware, actual application launches, device
cleanup or reset, packaged-GUI behavior, or physical end-to-end success.
RetroArch is **not** physically or fully end-to-end qualified.

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. The qualification grants no
physical cleanup authority (`not_authorized_for_recipe_qualification`), and no
ADB, ignored, host-sleep, identity-replacement, UI-smoke, packaged-GUI, or
operator qualification ran as part of this automated workflow.

## 6. Combined qualification boundary

Standalone BIOS qualification is recorded separately. Combined RetroArch + BIOS
qualification for the real `ayaneo.konkr_pocket_fit.base` default is recorded in
[the combined qualification document](recipe-qualification-retroarch-bios.md).
Obtainium and ROM/content qualification remain separate future work.
