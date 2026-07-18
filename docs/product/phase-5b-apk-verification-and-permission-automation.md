# Phase 5B — APK Manifest Inspection and Permission Automation

Status: in progress (Phases 5B1 through 5B8 complete)

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

Status: complete

The backend `inspectApk` capability now performs native crate-internal
inspection. Its strict request payload is:

```json
{
  "apkPath": "/path/to/selected.apk",
  "connectedDeviceApi": 35
}
```

`connectedDeviceApi` is optional. The request rejects unknown fields, including
the obsolete analyzer-fed `analyzer` and `facts` fields. Reusing the existing
capability name with the new payload and result is an intentional compatibility
break for analyzer-fed clients; the protocol version and capability name remain
unchanged. Frontend migration belongs to Phase 5B5.

The backend opens the selected APK through the Phase 5B2 bounded manifest
inspector and returns:

- `manifest`, containing package, version, and SDK metadata;
- `permissions`, containing every deterministic manifest declaration and, when
  device API context exists, its classification and applicability;
- review-only `runtimeGrantCandidates` and `appOpCandidates`;
- stable permission `warnings`;
- uppercase `calculatedSha256` metadata;
- `checksumStatus: "not_compared"` because no trusted expected checksum is
  accepted or compared in this phase;
- `signatureVerification: "not_performed"`.

Every candidate is derived only from applicable classifier automation metadata
and contains `selected: false`. Runtime candidates contain the permission name
and root requirement. App-op candidates additionally contain the reviewed
operation name and mode. These DTOs are review metadata, not shell commands;
Phase 5B4 constructs and executes no command.

When device API context is absent, every permission remains present with null
classification and applicability, both candidate lists are empty, and exactly
one `apk_permission_classification_context_unavailable` warning is returned.
The backend does not guess classifications or emit speculative per-permission
warnings.

With device context, applicable runtime-restricted, manual-special-access,
signature-or-privileged, and unknown permissions receive stable per-permission
warnings and are never candidates. Install-time permissions require no warning
or automation candidate. Every proven non-applicable permission receives one
`apk_permission_not_applicable` warning with the permission name and one of:

```text
declaration_requires_api_23
max_sdk_version_exceeded
permission_not_introduced
permission_replaced
target_sdk_below_minimum
```

Every indeterminate permission receives one
`apk_permission_applicability_indeterminate` warning with the permission name
and one of:

```text
invalid_max_sdk_version
target_sdk_unavailable
replacement_target_sdk_unavailable
```

Warnings, errors, and result metadata never expose the selected filesystem
path, parser diagnostics, raw enum diagnostics, or command strings. Manifest
failures retain the Phase 5B2 stable redacted reason codes; streamed file-hash
read failures use `apk_file_read_failed`.

The active inspection result contains no analyzer identity, analyzer evidence,
certificate fingerprint, signer metadata, or signature-verification claim.
The former analyzer inspection implementation was removed; its file retains
only the separate safe facts model consumed by existing draft generators.

All permission selections default to unchecked.

## Phase 5B5 — Permission review UI

Status: complete

The Config Editor app-generator flow now calls the native `inspectApk`
capability with its trusted, session-scoped APK path and an optional
connected-device API level. React retains only the opaque APK handle and the
safe native result. The former executable selection, persisted analyzer
preference, host process execution, output parsing, and analyzer-specific
errors are removed.

Every successful native inspection is stored separately from the conservative
facts passed to the existing draft generators, then draft generation continues.
The native result has no inspection-level blocking state. Missing device API
context, permission warnings, unsupported classifications, non-applicable or
indeterminate declarations, and empty candidate arrays are review information
and never block this transition. Existing draft and collision validation remain
independently authoritative.

The legacy generator facts use only native manifest evidence for package name,
version metadata, numeric SDK metadata, and requested permission names.
Application label, launcher activities, ABIs, and debuggable state remain
unavailable. The Config Editor continues to admit one standalone APK, so it
passes `split: false` and `base: true` as product-admission assumptions rather
than inspected evidence. Split APK sets remain unsupported.

The review UI displays manifest identity and SDK metadata, the locally
calculated SHA-256 with `checksumStatus: not_compared`, and
`signatureVerification: not_performed`. It states that no publisher checksum
comparison or APK signature verification occurred. Runtime grant candidates,
app-op candidates, and all other requested permissions are shown separately.
Runtime grants explain their `pm grant` and root requirements; root-dependent
app-ops warn that they may be unavailable on unrooted devices. Stable backend
warnings and structured applicability distinguish unavailable context,
not-applicable declarations, and indeterminate declarations.

All candidate checkboxes initialize from backend `selected: false` values.
Selections are reducer-owned review state only: a new inspection or APK/source
selection resets them, and they are never sent to draft, collision, save, or
sidecar requests. Phase 5B5 generates no permission step and persists no
selection.

## Phase 5B6 — Package-name enforcement

Status: complete

`install_apk` accepts one optional backward-compatible parameter:

```yaml
expected_package_name: com.example.app
```

The parameter accepts only a non-empty, non-whitespace string literal. It has no
default, and recipes that omit it retain the legacy installation path without
manifest inspection.

Generated pinned-remote-asset and latest-compatible-release recipes set this
field from the immutable package name in authoring-time APK inspection facts.
The editable app package field is not an enforcement source. Local APK and
remote user-provided-APK recipes omit the parameter because their runtime APK
may differ from the authoring sample. Missing or whitespace-only inspection
facts block pinned/latest recipe generation with
`apk_expected_package_name_unavailable`; the generator does not substitute the
editable app package.

After validating the resolved host file value, `.apk` extension, and file
existence, the executor uses the production bounded manifest parser to inspect
the resolved APK immediately before installation. Package names use exact,
case-sensitive equality without normalization, alias matching, or prefix
matching. A mismatch fails with `apk_package_mismatch`. A stable manifest
inspection failure fails with `apk_package_inspection_failed` and includes only
the corresponding stable manifest error code. Both failures occur before any
ADB install call and expose no APK path, parser diagnostic, or ZIP/AXML
implementation detail.

Do not add certificate, signer, or signature-verification parameters.

## Phase 5B7 — Optional checksum enforcement

Status: complete

`install_apk` accepts one additional optional, backward-compatible parameter:

```yaml
expected_sha256: ABCDEF...
```

The parameter accepts only a literal string containing exactly 64 hexadecimal
characters after trimming ASCII whitespace around the complete value. Upper-
and lowercase input is accepted, and valid input is normalized to 64 uppercase
hexadecimal characters in generated recipes and execution plans. Embedded
whitespace, prefixes such as `sha256:`, separators, non-hexadecimal characters,
references, non-string values, and incorrect lengths are invalid. The parameter
has no default, and recipes that omit it retain the legacy installation path
without calculating a checksum.

Immediately before installation, the executor streams the resolved host APK
through SHA-256 with a fixed-size buffer. Host runtime-value validation, the
`.apk` extension check, file existence, and optional package-name enforcement
all run before checksum calculation. When package-name and checksum enforcement
are both enabled, a package inspection or comparison failure prevents the
checksum file read. The device installation call occurs only after every enabled
check passes.

A mismatch fails before installation with `apk_checksum_mismatch` and reports
only the uppercase expected and actual hashes. A file open or read failure uses
`apk_checksum_read_failed`. These failures expose no APK path, filename, or raw
I/O diagnostic.

The remote app generator accepts a separate optional `trustedSha256` request
field. It belongs only to pinned-remote-asset recipe generation and is never
stored in app-definition metadata, tracking-source metadata, or inspection
facts. For a pinned asset, an omitted or blank value is accepted and omitted,
valid input is normalized and emitted as `expected_sha256`, and invalid input
blocks generation with `apk_trusted_sha256_invalid`. Latest-compatible-release
and user-provided-APK strategies accept an omitted or blank value but reject a
non-empty value with `apk_trusted_sha256_strategy_unsupported`.

The Config Editor exposes this field only for pinned assets and labels it as a
publisher-provided trusted checksum. Changing the source, APK, strategy, or
inspection session clears it. Editing it after review invalidates generated
recipe and collision results without clearing unrelated editable app or recipe
fields.

The authoring inspection result remains intentionally separate:

- `calculatedSha256` remains locally calculated metadata only;
- `checksumStatus` remains `not_compared`;
- the calculated value never initializes or supplies `trustedSha256`;
- absence of a trusted publisher checksum is not represented as verified.

Phase 5B7 adds no latest-release checksum discovery, checksum sidecar resolution,
release-text scraping, signature or certificate inspection, Java tooling,
Android Build Tools, Android SDK dependency, or authored-catalog changes.

## Phase 5B8 — Generated permission step

Status: complete

Selected applicable permission candidates generate one optional
`grant_permissions` step after installation and before first launch. Permission
automation is limited to `pinned_remote_asset` and
`latest_compatible_release` recipes because those strategies enforce the
package identity extracted from the inspected APK. Local APK and remote
`user_provided_apk` recipes accept no selection and reject a non-empty
selection instead of discarding it.

React submits only selected candidate identities and an opaque inspection
handle. Tauri reloads the native inspection stored for the session and APK,
requires the handle to match the currently stored inspection session, and
matches every submitted identity exactly. A non-empty selection is also
rejected when the currently trusted APK file identity no longer matches the
identity captured for that inspection. Package names, root requirements, and
commands are not accepted from React. Tauri forwards canonical literal-only
automation using `manifest.packageName` and the root requirement stored on each
matched candidate.

No selection preserves the existing generated recipe shape. A verified
selection produces one deterministic step immediately after `install_apk`.
When `launch_app` is present, it depends on the permission step; otherwise its
existing install dependency is unchanged.

The step requires `shell_command`, not blanket `root_shell`.

Runtime actions use `pm grant`. App-op actions use explicit allowlisted
mappings. Every action is emitted with `required: false`; only actions whose
stored candidate requires root include `when.rooted: true`. Runtime actions are
sorted by permission name. App-op actions are sorted by operation, mode, and
permission identity.

Default policy:

```yaml
policy:
  on_failure: warn
  require_all: false
```

An unrooted device may execute ordinary runtime grants while root-dependent app-op actions become `not_applicable`.

Candidate selections default to false. Reinspection or a source, APK,
installation-strategy, or device-API context transition clears them. Editing a
candidate invalidates draft, collision, and saved-review output without
clearing unrelated app or recipe form state. Initial automatic draft generation
passes no selections; explicit review and save requests submit only the current
selected identities.

## Phase 5B9 — Metadata and collision semantics

Phase 5B9 is complete. Every generated app definition owns one deterministic
`metadata.apk_inspection` record. Editable metadata is applied first, any
author-provided `apk_inspection` entry is removed, and the trusted generated
entry is appended. Unrelated metadata entries retain their insertion order.

The authored shape is:

```yaml
metadata:
  apk_inspection:
    package_name: com.example.app
    version_code: '42'
    version_name: 1.2.0
    min_sdk: 23
    target_sdk: 35
    calculated_sha256: 0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF
    checksum_status: not_compared
    signature_verification: not_performed
    requested_permissions:
    - name: android.permission.CAMERA
      declaration_kind: uses_permission
      max_sdk_version: null
      classification: runtime_grantable
      applicability:
        status: applicable
        reason: null
        maximum_sdk_version: null
        introduction_api: null
        minimum_device_api: null
        minimum_target_sdk: null
        target_sdk_state: null
    selected_runtime_permissions:
    - permission_name: android.permission.CAMERA
      requires_root: false
    selected_app_ops:
    - permission_name: android.permission.MANAGE_EXTERNAL_STORAGE
      operation_name: MANAGE_EXTERNAL_STORAGE
      mode: allow
      requires_root: true
```

All top-level keys and nested permission keys are always present; unavailable
optional values are `null`, and absent automation uses empty arrays. Requested
declarations are sorted and exactly deduplicated. Runtime selections sort by
permission name and root requirement. App-op selections sort by operation,
mode, permission identity, and root requirement. Local and user-provided APK
recipes never persist selected automation.

`calculated_sha256` is the locally calculated inspection digest, normalized to
uppercase. It is not publisher evidence. `checksum_status` remains
`not_compared`, and APK signatures remain unverified with
`signature_verification: not_performed`. A publisher-provided trusted SHA-256
exists only as `install_apk.params.expected_sha256`; it never changes or
supplies inspection metadata. The generator rejects malformed hashes, states,
permission enums, applicability combinations, and API bounds before emitting
authored YAML.

Collision review is performed from the app and complete recipe returned by the
sidecar during the same trusted Tauri generation request. React supplies only
an optional retained authored-root handle and cannot supply an app, recipe,
authored path, or fingerprint for collision analysis. Legacy backend requests
containing only `app` and `recipeId` retain the earlier ID, destination, source,
package, repository, release, and latest-policy checks without fingerprint
comparison.

For a comparable recipe, Rust requires exactly one `install_apk` step with a
literal non-empty `expected_package_name`, an optional literal valid
`expected_sha256`, an acyclic dependency graph, and no more than one downstream
`grant_permissions` step. Permission params must be literal and use the
expected package. Action identity includes the runtime permission or app-op,
mode, the executor's effective `required` value, and supported `when` fields:
`rooted`, `android_api_min`, and `android_api_max`. Permission policy includes
the effective `on_failure` and `require_all` values when actions exist.
References, malformed values, unknown condition keys, invalid API ranges,
conflicting duplicates, dependency ambiguity, or package-unenforced recipes
produce no comparable fingerprint.

Canonical fingerprints normalize trusted checksums to uppercase and sort
semantically identical actions, while ignoring names, descriptions, progress
text, step IDs, source URLs, and input ordering. Exact fingerprints across
different recipe IDs block with
`apk_security_automation_fingerprint_conflict`. A shared expected package with
different security or automation emits `apk_expected_package_overlap`. A
shared non-empty expected checksum under different expected packages emits
`apk_expected_sha256_overlap`. Exact matches suppress both warnings.

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
