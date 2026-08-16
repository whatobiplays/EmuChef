# Phase 6E.1 Recipe Qualification Foundation

**Owner:** EmuChef proper / Shared Runtime
**Status:** Automated qualification foundation complete for
`app.retroarch.provision`; Phase 6E is In progress; physical qualification
remains deferred.

## 1. Status and authority

Phase 6E.1 is the first automated recipe-qualification slice. The owner
explicitly deferred all remaining manual and physical qualification and
approved starting Phase 6E automated work without closing Phase 6D. Phase 6D
therefore remains **In progress** with every existing missing-evidence and
closure requirement unchanged; the deferral is a sequencing decision, not a
waiver. Ordinary production real execution remains disabled behind its existing
gating boundary.

## 2. Qualified automated boundary

The Phase 6E.1 foundation qualifies the real authored
`app.retroarch.provision` workflow through production code paths only:

- a strict qualification contract at
  `tests/fixtures/phase-6e/retroarch/qualification-contract.json` bound by
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
  expected action sections and authored waits, and sanitizes file-input
  summaries without leaking host parent paths.
- Artifact definitions enter the production resolution model at their exact
  default-cache filenames with authored URLs unchanged.
- The generated workflow completes successfully through the deterministic
  dry-run adapters with no Failed, Blocked, or Cancelled record.
- A repeated deterministic run skips the authored `install_retroarch` step
  through the production `package_installed` predicate while remaining
  successful.
- Omitting a required core-system directory fails the authored
  `copy_core_system_files` verification, retains prior completed results, and
  prevents the final launch from being reported as successfully executed.

## 4. What the tests do not prove

The deterministic foundation does not prove real download service availability,
APK acceptance on Android, Android permission/app-op behavior, private or
shared-storage semantics on hardware, actual application launches, device
cleanup or reset, packaged-GUI behavior, or physical end-to-end success.
RetroArch is **not** physically or fully end-to-end qualified.

## 5. Physical qualification disposition

Physical qualification is **Deferred by owner**. Phase 6E.1 grants no physical
cleanup authority (`not_authorized_in_phase_6e1`), and no ADB, ignored,
host-sleep, identity-replacement, UI-smoke, packaged-GUI, or operator
qualification ran as part of this slice.

## 6. Next automated recipe slices

Obtainium installation, BIOS copy, ROM/content copy, and canonical combined
device-plan workflows may be promoted as separate automated slices reusing this
contract and harness pattern. None of those workflows is marked started by
Phase 6E.1.
