# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

Native Apple Silicon macOS via Tauri/React and a trusted Rust/sidecar runtime.
The product is not a web-hosted service. Intel/universal macOS, Windows, Linux
and other packaging targets are outside the initial release scope.

## Users

### EmuChef proper

Nontechnical Android handheld owners configuring one connected supported device
through a guided setup, with visible recommendations, storage/content choices,
app-specific settings, mandatory review, supported staged execution and
truthful recovery/reporting.

### Config Editor

Technical authors maintaining App Definitions, Recipes, Device Profiles and
Device Plans. **New Config Editor feature work is deferred**; only narrowly
necessary compatibility maintenance preventing loss of new authored schema
fields is in the current program.

### Qualification operator

An operator who records bounded evidence for approved physical targets using
the existing product authority, not a separate privileged user pathway.

## Product Purpose

EmuChef proper makes Android emulation handheld installation and configuration
approachable, portable and reviewable. Simulation is optional. Real execution
is controlled by exact target/strategy and evidence, and remains disabled in
ordinary production pending explicit release authorization.

The end-user journey is **one guided setup** (UX-1 A), with additional reviews
when first-run defaults, new file bytes or OEM-script observations change the
trusted plan. Existing apps can be configured without reinstalling when safe.

## Positioning

App Definitions own intrinsic app facts and managed configuration rules;
Recipes define bounded workflows and exposed per-app settings; Device Profiles
and Plans determine capabilities and recommendations. The trusted Rust runtime
resolves all authority, files and device actions; React presents sanitized
projections and opaque handles.

Non-rooted devices with manufacturer-provided script runners may use a
**Device Profile-owned OEM execution strategy** with user-triggered privileged
scripts, ZIP outcome reports and alternating sequential ADB stages. The runner
never grants EmuChef direct ADB root. This is planned product behavior, not
present implementation or qualification.

## Operating Context

One Android device at a time on Apple Silicon macOS. Android SDK Platform-Tools
is user supplied; EmuChef does not bundle, download or silently acquire it.
Users may select logical internal/SD Storage Targets and Setup Locations,
ROMs/BIOS, a generated app configuration or a reusable template, and reviewed
copy policies. Saved intent never stores reusable execution authority.

A configuration file update preserves unrelated properties/comments where
supported and binds reviewed output and original state to the target. Per-file
original-byte backups are enabled by default, with per-file waivers after a
backup failure or an explicitly confirmed global opt-out. Restoration does not
imply whole-setup rollback.

## Capabilities and Constraints

- **Current implementation and accepted future requirements are distinct.**
  Runtime facts remain in `docs/architecture/` and original phase contracts.
  Product intent is normative in
  `docs/product/emuchef-requirements.md`; evidence obligations are tracked in
  `docs/product/qualification-obligations.md`.
- Mandatory review and immutable, scoped execution authority; simulation optional.
- Rust/Tauri alone holds device, filesystem, planning, backup, update and
  external-navigation authority. The frontend never holds raw device identities,
  executable paths, secrets or unredacted command logs.
- Real execution feature-gated and default-off for production. No implicit
  privilege escalation, background auto-replay, whole-device rollback or
  credentialed update delivery without separate approval.
- On AYN, Retroid and AYANEO, OEM runner *feature eligibility* is Profile-based
  and not restricted by firmware/version allowlists or per-model rollout. Exact
  physical configuration/usability claims still require matching evidence.
- Device- and operation-bound results must distinguish verified success,
  partial/unverified results, blocked dependencies and independent cleanup
  warnings. Cleanup is required when artifacts are no longer needed but a
  failed cleanup alone is not a failed setup (Q81 C).

## Brand Commitments

Use clear language for nontechnical users: app, device, storage choice, setup
and permissions rather than raw Recipe IDs, serials and runtime jargon.
Prefer truthful support information, explicit consent and actionable recovery
over unsupported automation.

## Evidence on Hand

The repository contains automated and historical physical evidence for
specific commits/workflows. It does **not** establish that the approved
configuration generator, OEM root-script path, corrected Mirror, broad
supported-device matrix or current EmuChef proper release is implemented or
physically qualified. Evidence contracts and outstanding repetitions are in
`docs/product/qualification-obligations.md`.

## Product Principles

1. A clear guided setup and exact review before mutation.
2. Portable, model-aware recommendations and trustworthy configuration composition.
3. One trusted runtime; no implicit file/device/frontend authority.
4. Verified outcomes, preserved recovery originals, bounded diagnostics and
   safe device cleanup.
5. Evidence before support or release claims; separate ownership for EmuChef
   proper, Config Editor and shared runtime.

## Accessibility & Inclusion

Keyboard-complete and screen-reader-operable flows, semantic landmarks,
accessible validation and progress, deterministic focus and safe fallback,
reduced motion, forced-colors, narrow-window and zoom resilience are
requirements of the end-user app and guided OEM stage handoffs.
