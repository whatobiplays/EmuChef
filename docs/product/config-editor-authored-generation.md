# Config Editor Authored Generation

## Status

Implemented current-state design. Typed authored foundations, standard
read-only device-profile generation, local APK generation, and public GitHub,
GitLab, Forgejo/Codeberg, or direct HTTPS APK source generation are implemented.
Extended device capability checks and authenticated/private source access remain
later work.

This document defines the Config Editor workflows for generating:

- a starter app definition and installation recipe from a GitHub source, remote APK, or local APK; and
- a starter device profile from a connected ADB device.

Rust is the sole product runtime. All analysis, validation, canonical generation, collision detection, and sidecar protocol behavior belong to `crates/emuchef-rust-backend`. Tauri owns native paths, trusted filesystem writes, configured external tools, and exact ADB serials. React owns presentation and explicit user choices.

## Product boundaries

Generation creates reviewable authored drafts. It does not silently alter the catalog.

The generated recipe becomes a normal recipe document and uses the existing recipe editor, validation, undo/redo, reference indexing, and canonical YAML path. App definitions and device profiles initially use dedicated draft forms and YAML previews rather than new persistent editor-session types.

Generation operations are side-effect free until an explicit save. Network analysis may retrieve bounded metadata and a selected APK into a generator-owned temporary workspace, but it does not install APKs, execute repository content, modify a device, or publish catalog files.

App definitions describe catalog and tracking metadata. They are not execution authority. The generated recipe remains the executable provisioning authority.

This scope does not generate device plans, infer configuration-copy behavior
from README prose, or automatically add unreviewed root, force-stop, app-data,
or configuration steps. Permission actions are generated only from explicit,
reviewed candidates described below.

## App and recipe generator

### Supported sources

The complete workflow supports:

1. local APK;
2. GitHub repository or release URL;
3. GitLab repository or release URL;
4. Forgejo-compatible repository or release URL, including Codeberg; and
5. direct remote APK URL.

Provider support uses documented release APIs and accepts only normalized repository and release identities. It does not scrape rendered HTML or implement a generic arbitrary-site crawler.

Source analysis collects bounded repository metadata, stable release metadata, and APK release assets. Drafts are excluded, prereleases are excluded by default, and multiple eligible APK assets require explicit selection.

### APK inspection

The Rust backend opens the selected APK as a bounded ZIP input and inspects its
binary `AndroidManifest.xml`. It extracts the package name, version code,
version name, minimum SDK, target SDK, and deterministic permission
declarations, including declaration kind and `maxSdkVersion`. Inspection
failures use stable reason codes and redact paths, filenames, parser details,
and other unsafe diagnostics.

The native inspection does not extract the application label, launcher
activities, supported ABIs, or debuggable state. The current product admits one
standalone APK and therefore passes `split: false` and `base: true` to draft
generation as admission assumptions, not manifest-derived proof. Split APK,
APKS, and AAB workflows are unsupported.

This workflow does not invoke or require Java, Android SDK Build Tools,
`apkanalyzer`, `aapt2`, or an installed Android SDK. Inspection never installs
the package.

The inspection result includes an uppercase `calculatedSha256`, which is a
locally calculated digest of the selected APK file. It remains separate from
publisher or author trust: `checksumStatus` is `not_compared` and
`signatureVerification` is `not_performed`. EmuChef performs no APK v1, v2,
v3, or v4 signature verification, signer-certificate extraction, certificate
pinning, proof-of-rotation processing, or signer identity validation.

GitHub names, topics, descriptions, filenames, release metadata, and the local
digest may inform review but may not substitute for manifest package evidence
or an explicitly trusted publisher checksum.

### Installation strategies

The wizard exposes three source-neutral recipe strategies when the selected source supports them.

#### Pinned release

This is the default for a selected release asset or direct HTTPS APK. The
recipe declares a `remote_file` artifact with the exact asset URL, resolves it,
and installs it. The generated install step enforces the package name extracted
during inspection. When the author supplies an explicitly trusted publisher
SHA-256, the generator also emits it as `expected_sha256`; the locally
calculated inspection digest never supplies that field automatically.

#### Latest compatible release

This is available for public GitHub, GitLab, and Forgejo-compatible repository sources. The author selects one APK from the current release and EmuChef derives an editable filename regular expression by generalizing version-like segments while preserving variant, platform, and architecture text. The rule is previewed against the selected release and review is blocked unless it matches exactly one APK.

The generated recipe uses explicit `resolve_remote_release` and
`download_remote_file` steps. Runtime resolution dispatches by provider,
excludes drafts or unpublished releases, excludes prereleases unless the author
enables them, orders releases deterministically, and fails safely when the saved
rule matches zero or multiple assets. The install step enforces the inspected
package name against the resolved APK. Latest-compatible generation does not
persist the authoring-time calculated digest as a trusted expected checksum.

#### User-provided APK

This is the default for a local APK and is available for any source. The recipe
declares a required `file` input with role `apk` and installs that input. It
never persists the developer's local absolute APK path. Because the runtime APK
may differ from the authoring sample, this strategy omits package/checksum
enforcement and permission selections derived from that sample.

### Generated recipe

The starter recipe is intentionally minimal:

- optional `resolve_artifacts` for a pinned remote source;
- optional `resolve_remote_release` followed by `download_remote_file` for latest-compatible provider sources;
- `install_apk` constrained by `apk_install`;
- `package_installed` skip condition using the reviewed package name;
- optional package and checksum enforcement as described above; and
- an optional `grant_permissions` step for reviewed eligible selections.

The generator uses the existing recipe model and step specifications. It does not maintain a second recipe representation.

Immediately before installation, `expected_package_name` causes the executor to
reinspect the resolved APK with the bounded Rust manifest parser and compare the
package exactly and case-sensitively. `expected_sha256`, when present, causes a
streamed file digest comparison. A mismatch or redacted inspection/read failure
prevents the ADB install call. Both parameters are optional so legacy and
user-provided recipes retain their previous behavior when the fields are
absent. These checks are integrity controls, not APK signature verification.

The current native inspection supplies no launcher activity, so the normal
Config Editor flow does not derive a launch step from the inspected APK.

### Permission automation

Permission generation is independent of a connected device. The backend
classifies exact reviewed permission names as runtime-grantable,
runtime-restricted, app-op-grantable, manual special access, install-time,
signature-or-privileged, or unknown. The backend registry owns platform
introduction, maximum, restriction, target-SDK, and explicit automation
metadata. Manifest declaration kind, numeric `maxSdkVersion`, and inspected
target SDK refine candidate bounds. Missing or non-numeric target SDK data
fails closed only for rules that require it.

Device API bounds and application target-SDK cutoffs remain separate.
`WRITE_EXTERNAL_STORAGE` is applicable and eligible for a runtime candidate
only when the application target SDK is numeric and at most 29. Its candidate
starts at Android API 23 and has no catalog device maximum, although a numeric
manifest `maxSdkVersion` may cap it. Target SDK 30 or newer is non-applicable
because it exceeds the reviewed maximum target SDK; `minSdkVersion` does not
alter that decision. Missing or non-numeric target SDK remains indeterminate
and fails closed.

Only explicit selections matched back to the trusted native inspection may
produce automation, and only package-enforced pinned or latest-compatible
recipes are eligible. Runtime actions use `pm grant`; the initial supported
app-op maps `MANAGE_EXTERNAL_STORAGE` to mode `allow`. Generated actions use
the inspected expected package, set `required: false`, and use:

```yaml
policy:
  on_failure: warn
  require_all: false
```

The step requires `shell_command`, never blanket `root_shell`. Root and Android
API applicability are action-scoped through supported `when.rooted`,
`when.android_api_min`, and `when.android_api_max` conditions. Current
generation emits a non-null API minimum for every action, an API maximum only
when catalog or manifest facts provide one, and `rooted: true` only for a
selected action whose reviewed catalog entry requires root. The executor
evaluates these conditions against the actual target device and marks an unmet
condition `not_applicable` without suppressing eligible actions.

The exact-name catalog covers reviewed public dangerous permissions but does
not infer platform behavior for arbitrary names. Runtime-restricted,
signature/privileged, unknown, role-based,
accessibility, VPN, notification-listener, device-admin, Settings-mediated, and
other manual special-access cases remain warning-only, unsupported, or manual
as applicable.

The permission declaration list is the authoritative presentation for
non-candidate classification and applicability outcomes. Permission-specific
backend warnings are not repeated as warning cards. The warning-card section
contains only inspection-wide warnings with no permission name and is omitted
when none remain, while unknown and custom permission declarations remain
visible once under Other requested permissions.

### Generated app definition

The app definition preserves schema version 1 and the existing authored shape under `authored/apps`:

- identity and display metadata;
- primary package and aliases;
- free-form install-source metadata;
- free-form tracking-source metadata;
- artifact support declarations;
- provisioning metadata;
- input metadata; and
- extensible metadata.

`install_source` and `tracking_source` describe source and tracking intent. They do not dynamically resolve recipe artifacts.

Every generated app definition owns the reserved
`metadata.apk_inspection` record. Generation removes any editable value under
that key, writes the trusted inspection metadata, and preserves unrelated
author metadata. Local and user-provided flows store empty selected-action
arrays. See
[Phase 5B](phase-5b-apk-verification-and-permission-automation.md) for the exact
field shape and validation rules.

### Evidence

Each proposed value carries one of these confidence states:

- `verified`: supplied by APK manifest or exact source metadata;
- `derived`: deterministically transformed from verified data;
- `suggested`: heuristic authoring recommendation; or
- `missing`: no value is proposed.

The draft response includes provenance and warnings. Final YAML contains authored values, not evidence wrappers.

### Collision detection

Before saving, Rust scans the selected authored root for:

- duplicate app ID;
- duplicate primary package;
- duplicate recipe ID;
- duplicate destination path;
- existing repository metadata;
- existing pinned asset URL; and
- an identical latest-release policy fingerprint composed of provider, base URL, repository, asset pattern, and prerelease policy; and
- overlapping package/checksum enforcement or an identical APK
  security-and-permission fingerprint under another recipe ID.

ID, path, identical latest-policy, and identical APK security-automation
fingerprint conflicts are blocking. Other package, checksum, or repository
overlap is a warning requiring review. Rust derives the security fingerprint
from the complete generated recipe returned by the trusted generation request;
React cannot supply the recipe or a fingerprint. Legacy collision requests
without a complete recipe retain their earlier checks. See
[Phase 5B](phase-5b-apk-verification-and-permission-automation.md) for exact
comparability and fingerprint semantics.

## Device profile generator

### Existing runtime reuse

The workflow reuses the existing sidecar operations and ADB implementation:

- `listAdbDevices`;
- `probeDevice`; and
- the existing detected-device fact and profile-matching logic.

The Config Editor adds trusted Tauri wrappers and frontend projections for those operations rather than creating another ADB process runner.

### Standard capture

Standard capture is the default and performs no device writes. It collects only facts useful for authored matching and diagnostics:

- manufacturer;
- brand;
- model;
- product;
- device;
- board;
- hardware;
- ABI list;
- Android major version; and
- Android API level.

The exact ADB serial remains trusted transport state. It is not included in generated YAML, React persistence, or normal logs.
The Config Editor resolves the standard probe executable as literal `adb` from
`PATH`; executable selection and Android SDK environment discovery are deferred.

### Extended capability checks

Extended checks are deferred and are not part of the implemented standard
device-profile generator. A later explicit action may test:

- temporary shared-storage write, read, and cleanup;
- package-manager command availability;
- activity-manager command availability; and
- optional root-shell access through `su -c id`.

The UI states the exact checks before execution. Root probing never runs automatically. The first implementation does not install a probe APK.

### Capability defaults

Generated profiles contain all current capability-default fields. Evidence remains separate from the final booleans.

A successful standard ADB probe verifies `adb_available` and `shell_command`. Other normal-device capabilities may be suggested conservatively. `package_remove_for_user`, `root_shell`, and `app_data_write` default to false unless explicitly supported by evidence and author choice.

### Match generation

Generated matching is conservative:

- manufacturer and brand use detected exact tokens in their respective contains lists;
- the model is regex escaped and emitted as an anchored exact pattern;
- Android major version becomes `android_version.min`;
- no Android maximum is generated;
- no alternate model, OEM alias, product-family pattern, or generic vendor regex is invented.

The user can edit every proposed match field before saving.

The default profile ID is `<normalized-manufacturer>.<normalized-model>`. The author may change it before save.

## Backend architecture

New implementation belongs under a focused Rust module family:

```text
crates/emuchef-rust-backend/src/generation/
  mod.rs
  diagnostics.rs
  identifiers.rs
  collisions.rs
  app.rs
  app_definition.rs
  recipe.rs
  github.rs
  apk.rs
  device_profile.rs
  device_capabilities.rs
```

The implementation reuses existing `device_probe`, `end_user_runtime`, `model`, `step_specs`, `validation`, `yaml`, and catalog behavior. Authoring-time GitHub/APK analysis does not belong in the execution artifact resolver.

## Typed authored models

Before generation ships, Rust owns typed schema-v1 models for app definitions and device profiles. These models provide:

- structural parsing;
- canonical YAML emission;
- stable validation;
- regex and Android-range validation for device profiles;
- package and source-shape validation for app definitions; and
- the common authority used by generator output, save validation, catalog loading, and future dedicated editors.

Generated YAML is never treated as an unvalidated `serde_json::Value` blob.

Schema-v1 parsing rejects unknown fields in fixed top-level and nested
structures. Extensibility is limited to `install_source.options`, fields after
`tracking_source.type`, and `metadata`; these mappings retain nested
JSON-compatible values and insertion order without silently discarding data.
Authored IDs use lowercase alphanumeric segments separated by `.`, `_`, or
`-`.

Canonical YAML emits fixed fields in schema order and emits empty collection
fields explicitly. Optional scalar values and optional Android range bounds are
omitted when absent. Re-emitting canonical YAML is byte-stable, while ordered
extension mappings retain their authored order.

## Sidecar protocol

Negotiated capabilities include:

- `analyzeAppSource`;
- `inspectApk`;
- `generateAppRecipeDraft`;
- `generateDeviceProfileDraft`; and
- `checkGeneratedCatalogCollisions`.

Existing `listAdbDevices` and `probeDevice` capabilities are reused.

Analysis and draft generation return structured data and perform no
authored-data writes. Backend responses use stable error codes and redact
credentials, exact serials, absolute paths, parser diagnostics, and unsafe
network details from product-facing errors.

## Tauri and save ownership

Tauri owns:

- native local-APK selection;
- session-scoped native APK handles and file identities;
- exact ADB serials and device handles;
- native save destinations;
- final collision revalidation; and
- trusted writes.

Saving an app and recipe is one logical operation. Both drafts are validated before publication. Temporary files are written and synced before either final path is published. Existing files are never overwritten without explicit approval. If final publication partially fails, Tauri removes any newly published counterpart when safe and reports the incomplete outcome.

After a successful app-and-recipe save, the generated recipe opens through the existing recipe document session.

Device-profile roots and device serials use handles scoped to one ephemeral
generator session. React receives no native root path. A profile save
revalidates and rescans immediately, rejects incomplete collision scans and
existing destinations, writes and syncs a temporary sibling, and publishes with
atomic create-new/no-clobber semantics.

## Config Editor workflow

Top-level actions:

```text
File
  Generate App and Recipe...
  Generate Device Profile...
```

App workflow:

1. choose source;
2. inspect repository or release;
3. select APK when needed;
4. review APK facts;
5. configure app draft;
6. configure recipe draft;
7. review YAML and collisions; and
8. save and open the recipe.

Device workflow:

1. list and select one connected device;
2. review detected facts;
3. configure match criteria;
4. review capability defaults;
5. review YAML and collisions; and
6. save the profile.

Generator state is separate from `RecipeDocumentDto`. A sidecar restart invalidates active analysis and requires the wizard to rerun it.

## Security requirements

- GitHub mode permits only validated HTTPS GitHub/API origins.
- Network bodies, redirects, transfer duration, and APK size are bounded.
- HTTPS-to-HTTP downgrade is rejected.
- Repository content and downloaded scripts are never executed.
- README HTML is not rendered or interpreted as execution instructions.
- GitHub credentials, when added later, remain in trusted OS-backed storage and never cross into React.
- APK filenames, manifest strings, repository fields, and parser input are
  untrusted.
- APK inspection stays inside the bounded native Rust parser boundary.
- Standard device capture performs no writes.
- Any future extended checks require explicit user action and cleanup of temporary material.

## Delivery phases

### Phase 1: typed authored foundations

- typed `AppDefinitionV1` and `DeviceProfileV1` models;
- structural parsing and canonical emission;
- focused validation;
- identifier normalization;
- collision classification; and
- generation evidence/diagnostic DTO foundations.

### Phase 2: device profile generator

- Config Editor access to existing device listing and probing;
- expanded safe device facts;
- profile draft generation;
- collision checking;
- native profile save; and
- fake-runner tests.

### Phase 3: local APK generator

- native bounded APK manifest inspection;
- native APK picker;
- APK inspection;
- app-definition draft;
- user-provided-APK recipe draft;
- dual-document validation and save; and
- opening the generated recipe.

The implemented local workflow uses explicit BYO metadata:
`install_source.type` is `user_provided_apk`, its resolver is `none`,
`tracking_source.type` is `local_apk`, direct APK support is not required, and
BYO APK support is required. Native APK facts are review evidence and are
persisted under the generator-owned `metadata.apk_inspection` key; local flows
persist no selected permission actions.

Native selection accepts regular APK files up to 2 GiB. The backend reads one
root `AndroidManifest.xml` through its bounded ZIP and binary-XML parser and
streams the APK file to calculate the local SHA-256. It does not execute an
analyzer process or expose certificate or signer facts.

The Config Editor enforces one active generator wizard. App-generator paths and
file identities remain behind session-scoped Tauri handles. Final dual-file
publication revalidates and rescans, uses synced temporary siblings and
create-new hard-link publication, removes the first publication when the
second fails and rollback is safe, then opens the recipe in the existing
document session.

### Phase 4: Provider-hosted and remote APK sources

- repository and release analysis;
- stable asset filtering and selection;
- bounded temporary APK download;
- pinned-release recipe generation;
- direct remote APK mode; and
- network, timeout, and redaction tests.

The implemented workflow uses provider release APIs for GitHub, GitLab, and
Forgejo-compatible hosts. Drafts or unpublished releases are excluded,
repository prereleases are opt-in, exact prereleases require confirmation, and
eligible assets are non-empty `.apk` files no larger than 2 GiB. Direct URLs
must use public HTTPS without credentials, fragments, or query parameters.
Metadata bodies are bounded to 2 MiB, redirects to five safe HTTPS hops,
connections to 10 seconds, and requests to 30 seconds. Downloads stream into a
session-owned temporary workspace and use the same native Rust inspection.

GitHub public API access is unauthenticated. GitHub's unauthenticated rate
limits can temporarily block repository or release analysis. The Config Editor
reports a bounded advisory retry indication when GitHub supplies valid numeric
retry or reset metadata; the indication does not guarantee that a later request
will succeed. GitHub authentication remains future refinement work.

Pinned generation stores normalized source identity and emits a `remote_file`
artifact plus resolve/install steps. Authors may instead choose the existing
user-provided APK strategy, which preserves the Phase 3 `user_provided_apk`
and `local_apk` source shape without remote tracking fields. Authentication,
private repositories, arbitrary-site scraping, background refresh, and
split-package formats remain excluded.

### Phase 5: GitHub release-pattern testing

GitHub repository sources using the latest-compatible strategy expose an
immediate filename-pattern preview after source analysis. GitHub analysis
requests at most 30 releases. The trusted session retains every non-draft
release in provider response order, including prereleases and releases with no
eligible APK assets. The Include prereleases selection filters this retained
set locally and does not make another network request.

The preview applies the unmodified author-entered regular expression to each
release's eligible APK filenames using substring-search semantics unless the
expression itself supplies anchors. It reports `unique_match`, `no_match`, or
`multiple_matches` for each eligible analyzed release and sorts displayed
matching filenames deterministically. Summary counts cover the complete
eligible retained set. The UI displays at most the first 10 rows in provider
response order.

For this check, the current release is the first retained release after draft
exclusion and prerelease filtering. This is provider response order; the
workflow does not claim that GitHub guarantees chronological ordering and does
not reorder releases by tag, semantic version, filename, or parsed timestamp.
The current release must contain exactly one match. No trusted analysis, an
empty analysis, no releases after prerelease filtering, zero current-release
matches, multiple current-release matches, and invalid Rust regex syntax are
blocking. Older zero-match and multiple-match results remain visible warnings
when the current release has one match.

The browser preview uses JavaScript regular-expression behavior for immediate
feedback and is not final validation. Tauri ignores browser-computed ordering,
counts, outcomes, and release contents. It constructs a minimal ordered
snapshot from session-owned analysis containing only release tags, prerelease
flags, and eligible APK filenames. Rust evaluates the raw pattern again with
the production `regex` engine before draft generation and saving. A pattern
accepted by JavaScript but rejected by Rust is blocking. Pinned remote assets,
direct APK sources, and user-provided APK strategies do not require or consume
release-pattern results and retain their existing generated recipe shapes.

### Later refinement

Potential later work includes OS-keychain GitHub credentials, dedicated
app/profile editors, Obtainium import, source-update checks, aliases, and
device-plan assistance.

## Verification

Rust tests cover typed parsing/emission, identifier normalization, regex
escaping, Android ranges, evidence assignment, collision classification,
generated recipe validity, native manifest parsing, permission classification,
install-time package/checksum enforcement, URL parsing, release filtering,
timeouts, and redaction.

Protocol tests cover capability negotiation, malformed requests, side-effect-free generation, stable errors, and absence of unsafe paths or serials.

Config Editor tests cover wizard transitions, cancellation, ambiguous APK selection, backend restart invalidation, collision blocking, dirty-document protection, save recovery, and opening the generated recipe.

Normal automated tests use local APK fixtures, fake GitHub HTTP endpoints, and fake ADB runners. They do not require public GitHub access, a real device, or installed Android SDK tools.
