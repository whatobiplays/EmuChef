# Phase 5B — APK Manifest Inspection and Permission Automation

Status: in progress (Phases 5B1 through 5B3 complete)

## Purpose

Phase 5B adds lightweight APK manifest inspection and optional permission automation for generated app recipes. It deliberately excludes APK cryptographic signature verification and does not bundle Java, Android Build Tools, or another external verifier runtime.

## Product decisions

1. Parse packaged `AndroidManifest.xml` with a small Rust dependency.
2. Extract package identity, version metadata, SDK metadata, and requested permissions.
3. Enforce expected package-name matching for generated remote APK recipes.
4. Support publisher-provided SHA-256 checksums when trustworthy checksum metadata is available.
5. Do not inspect, validate, pin, or display APK signer certificates.
6. Do not claim APK signature verification.
7. Do not bundle a JVM, `apksigner`, Android Build Tools, or require an installed Android SDK.
8. Permission automation is author-opt-in and disabled by default.
9. Ordinary runtime permissions use `pm grant` without requiring root.
10. Explicitly allowlisted special permissions may use app-ops; root is scoped to individual actions.
11. Start the app-op allowlist with `MANAGE_EXTERNAL_STORAGE` on Android API 30+.
12. Unknown, restricted, privileged, role-based, and Settings-mediated permissions are displayed but not automatically granted.
13. Existing authored catalog files are not modified as part of this phase.

## Trust model

EmuChef may report:

- source provider and repository;
- selected release and asset;
- HTTPS download;
- calculated download SHA-256;
- publisher checksum verification when available;
- manifest package-name match;
- manifest permission review;
- APK signature status: not verified by EmuChef.

Package matching and checksums are useful integrity controls, but they are not equivalent to Android signature verification. User-facing copy must not imply otherwise.

# Phase plan

## Phase 5B1 — Manifest parser qualification

Status: complete

Qualify a lightweight Rust Android binary XML parser for hostile APK input without adding signature-verification dependencies.

The qualification module must:

- open the APK as a ZIP;
- read only `AndroidManifest.xml`;
- enforce a decompressed-size limit;
- catch parser panics where necessary;
- map failures to stable redacted errors;
- extract package name, version code/name, min SDK, target SDK, and requested permissions;
- parse `uses-permission` and `uses-permission-sdk-23`;
- preserve `android:maxSdkVersion`;
- sort and deduplicate permission declarations deterministically.

During qualification, the parser was the exact, test-only development
dependency `rusty-axml = { version = "=0.2.1", default-features = false }`.
The qualification module was compiled only for tests until Phase 5B2 promoted
the same wrapper and dependency into production backend compilation. No
`apksig` dependency is permitted.

Exit criteria:

- malformed ZIP, missing manifest, malformed binary XML, oversized manifest, and missing package fail safely;
- returned errors contain no absolute paths, URLs, filenames, or raw parser diagnostics;
- focused tests pass;
- the parser is acceptable for production manifest extraction.

Qualification result: `rusty-axml 0.2.1` qualifies as the parser candidate for
later production manifest extraction only when retained behind the tested
defensive wrapper. Phase 5B1 does not promote that wrapper into production.

The wrapper requirements established by qualification are:

- accept exactly one root ZIP entry named `AndroidManifest.xml`, including an
  entry-count check that detects duplicate names collapsed by the ZIP reader;
- reject manifests whose declared decompressed size exceeds the selected 4 MiB
  defensive limit and enforce the same limit while reading;
- validate the binary XML root chunk type, header size, and declared total size
  before parsing;
- contain parser and traversal panics and discard panic payloads and third-party
  diagnostics;
- accept only tested typed decimal and hexadecimal integer renderings and reject
  unknown or malformed typed metadata;
- inspect permission declarations only when they are direct manifest children.

The remaining parser risks are its known panic paths on malformed chunks, its
permissive handling of some malformed binary XML, and its string-rendered typed
values. These behaviors make the structural checks, integer validation, and
panic boundary mandatory. The dependency also brings its own test-only ZIP
parser version transitively, and the deterministic generated qualification
fixtures do not replace broader real-world APK corpus testing before production
promotion.

## Phase 5B2 — Stable APK manifest model

Status: complete

The qualified wrapper is now a production-compiled, crate-internal backend
module. `rusty-axml = { version = "=0.2.1", default-features = false }` is an
exact-pinned normal dependency. The module, inspection function, errors, facts,
permission declarations, and declaration-kind enum use `pub(crate)` visibility;
they are not public crate API, protocol DTOs, executor inputs, or frontend
types. No production caller is integrated in Phase 5B2.

The stable internal facts are:

- a required package name;
- optional version code and version name;
- optional minimum and target SDK versions;
- requested permissions with declaration kind and optional `maxSdkVersion`.

Known numeric metadata is represented as canonical decimal strings. SDK
codenames remain strings where Android permits them. `rusty-axml` types and
diagnostics remain private to the module.

The Phase 5B1 wrapper remains mandatory because the parser has known panic
paths, permissive handling of some malformed binary XML, and string-rendered
typed values. Production compilation preserves exact-one root
`AndroidManifest.xml` handling, ZIP entry-count consistency checks, the 4 MiB
declared and streamed decompression limits, AXML container validation,
parser/traversal panic containment, direct-child-only extraction, tested
integer normalization, deterministic permission sorting, exact deduplication,
and stable redacted failures. A second direct-child `uses-sdk` declaration is
invalid regardless of whether either declaration is empty or partial. Nested
`uses-sdk` elements are ignored.

Stable errors should include:

```text
apk_zip_invalid
apk_manifest_missing
apk_manifest_too_large
apk_manifest_invalid
apk_package_missing
apk_manifest_inspection_failed
```

Package-name enforcement and `apk_package_mismatch` belong to Phase 5B6. The
permission catalog and classification, checksums, install/executor integration,
protocol and UI surfaces, and authored-recipe behavior remain deferred. APK
signature verification remains an explicit non-goal; EmuChef does not add
signer inspection, certificates, Java, `apksigner`, Android Build Tools, or an
Android SDK dependency.

## Phase 5B3 — Android permission catalog and classifier

Status: complete

Classify requested permissions as:

- `RuntimeGrantable`;
- `RuntimeRestricted`;
- `AppOpGrantable`;
- `ManualSpecialAccess`;
- `InstallTime`;
- `SignatureOrPrivileged`;
- `Unknown`.

Classification must consider declaration kind, `maxSdkVersion`, app target SDK, connected-device API, permission introduction/replacement API, and explicit app-op mappings.

Do not infer protection level or app-op identity from permission-name patterns.

Initial explicit mapping:

```text
android.permission.MANAGE_EXTERNAL_STORAGE
  -> MANAGE_EXTERNAL_STORAGE
  -> mode allow
  -> API 30+
  -> root required for the action
```

The production backend now owns a crate-internal exact-name catalog and pure
classifier. The stable internal result preserves the original manifest
declaration and records classification separately from applicability. An
applicable result may carry reviewed future automation metadata; non-applicable
or indeterminate results never carry actionable metadata. No ADB command,
executor, recipe-generation, protocol, Tauri, or frontend integration exists in
Phase 5B3.

The intentionally small initial catalog contains `CAMERA`,
`BODY_SENSORS_BACKGROUND`, `MANAGE_EXTERNAL_STORAGE`, `SYSTEM_ALERT_WINDOW`,
`INTERNET`, `WRITE_SECURE_SETTINGS`, `READ_EXTERNAL_STORAGE`, and
`READ_MEDIA_IMAGES`. These entries prove the seven classification categories,
runtime-permission target-SDK handling, introduction boundaries, compound
replacement conditions, and the sole initial app-op mapping. Every other exact
permission name classifies as `Unknown` and has no automation metadata; the
classifier never infers behavior from a prefix or substring.

Applicability evaluates the declaration kind, `maxSdkVersion`, connected-device
API, cataloged introduction data, application target SDK, and cataloged
replacement data. `uses-permission-sdk-23` is inapplicable below API 23, and a
declaration expires only when its numeric `maxSdkVersion` is lower than the
device API. Missing, codename, or otherwise non-numeric target SDK data fails
closed whenever a rule requires a numeric target: the result becomes
indeterminate, exposes no automation metadata, and does not assert an
unsupported replacement decision.

The Android 13 storage transition is modeled as a compound catalog rule.
`READ_EXTERNAL_STORAGE` is replaced by granular media permissions only when the
connected device is API 33 or newer and the numeric application target SDK is
33 or newer. Either threshold below 33 leaves its ordinary catalog
classification applicable. On API 33 or newer, unavailable numeric target SDK
data is indeterminate rather than being treated as proof of replacement.
`READ_MEDIA_IMAGES` is runtime-grantable only when both the connected device and
numeric application target SDK are API 33 or newer.

`MANAGE_EXTERNAL_STORAGE` remains the only Phase 5B3 app-op entry. It maps
exactly to app-op `MANAGE_EXTERNAL_STORAGE`, mode `allow`, with root required
for the future action, and is non-applicable below API 30. Phase 5B3 does not
construct or execute that future action.

## Phase 5B4 — Authoring inspection result

Expose safe structured fields:

- package and version metadata;
- SDK metadata;
- complete requested permissions;
- deterministic classifications;
- candidate `pm grant` actions;
- candidate app-op actions;
- manual/unsupported warnings;
- calculated APK SHA-256;
- publisher checksum status when available;
- `signatureVerification: "not_performed"`.

All permission selections default to unchecked.

## Phase 5B5 — Permission review UI

Show separate sections for runtime permissions, app-op-backed permissions, and manual/unsupported permissions.

The UI must explain that:

- runtime grants use `pm grant` and do not require root;
- root-dependent app-ops are skipped when root is unavailable;
- unknown or unsupported permissions are not automated;
- EmuChef does not cryptographically verify APK signatures.

## Phase 5B6 — Package-name enforcement

Extend `install_apk` with one optional backward-compatible parameter:

```yaml
expected_package_name: com.example.app
```

Generated remote pinned/latest recipes set this field from authoring-time manifest inspection. Immediately before installation, the executor reinspects the resolved APK and rejects a mismatch before ADB install.

Do not add certificate, signer, or signature-verification parameters.

## Phase 5B7 — Optional checksum enforcement

Where a provider or publisher exposes a trustworthy checksum, support:

```yaml
expected_sha256: ABCDEF...
```

Rules:

- normalize to uppercase hexadecimal internally;
- stream SHA-256 calculation;
- reject mismatch before installation;
- do not scrape arbitrary release prose unless checksum format and asset binding are unambiguous;
- absence of a publisher checksum is not represented as verified;
- a locally calculated hash is metadata unless compared with trusted expected data.

This phase may be deferred independently.

## Phase 5B8 — Generated permission step

Generate one optional `grant_permissions` step after installation and before first launch.

The step requires `shell_command`, not blanket `root_shell`.

Runtime actions use `pm grant`. App-op actions use explicit allowlisted mappings and may include `when.rooted: true`.

Default policy:

```yaml
policy:
  on_failure: warn
  require_all: false
```

An unrooted device may execute ordinary runtime grants while root-dependent app-op actions become `not_applicable`.

## Phase 5B9 — Metadata and collision semantics

Persist safe manifest and permission metadata, including package name, target SDK, calculated SHA-256, checksum status, and `signature_verification: not_performed`.

Collision fingerprints should include expected package name, trusted expected checksum when present, selected runtime permissions, selected app-op actions, and API/root conditions.

## Phase 5B10 — Tests

Cover:

- package/version/SDK extraction;
- both permission declaration kinds;
- `maxSdkVersion`;
- deterministic ordering and deduplication;
- malformed/missing/oversized manifests;
- redacted errors;
- permission classification and API filtering;
- runtime-only, app-op-only, and mixed generated steps;
- no blanket root capability;
- package mismatch rejection;
- checksum mismatch rejection when enabled;
- legacy recipe compatibility;
- mixed permission action behavior.

## Phase 5B11 — Documentation and release gates

Document manifest inspection, package-name enforcement, checksum semantics, absence of APK signature verification, permission classification, runtime grants, app-op behavior, root-scoped conditions, and unsupported/manual special access.

Run the normal backend, Tauri, frontend, lint, snapshot, and `git diff --check` gates before release.

# Delivery order

1. Manifest parser qualification.
2. Stable manifest model.
3. Permission catalog and classifier.
4. Authoring DTO and UI.
5. Package-name enforcement.
6. Optional checksum enforcement.
7. Generated permission step.
8. Metadata, collisions, documentation, and regression coverage.

# Non-goals

- APK v1/v2/v3/v4 cryptographic signature verification.
- Signer certificate extraction or pinning.
- Proof-of-rotation handling.
- Bundled Java runtime or Android Build Tools.
- Installed Android SDK dependency.
- Split APK/APKS/AAB support.
- Automatic approval of role-, accessibility-, VPN-, or Settings-mediated access.
