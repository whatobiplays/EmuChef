# Config Editor Authored Generation

## Status

Implemented current-state design. Typed authored foundations, standard
read-only device-profile generation, local APK generation, and public GitHub or
direct HTTPS APK source generation are implemented. Extended device capability
checks and authenticated/private source access remain later work.

This document defines the Config Editor workflows for generating:

- a starter app definition and installation recipe from a GitHub source, remote APK, or local APK; and
- a starter device profile from a connected ADB device.

Rust is the sole product runtime. All analysis, validation, canonical generation, collision detection, and sidecar protocol behavior belong to `crates/emuchef-rust-backend`. Tauri owns native paths, trusted filesystem writes, configured external tools, and exact ADB serials. React owns presentation and explicit user choices.

## Product boundaries

Generation creates reviewable authored drafts. It does not silently alter the catalog.

The generated recipe becomes a normal recipe document and uses the existing recipe editor, validation, undo/redo, reference indexing, and canonical YAML path. App definitions and device profiles initially use dedicated draft forms and YAML previews rather than new persistent editor-session types.

Generation operations are side-effect free until an explicit save. Network analysis may retrieve bounded metadata and a selected APK into a generator-owned temporary workspace, but it does not install APKs, execute repository content, modify a device, or publish catalog files.

App definitions describe catalog and tracking metadata. They are not execution authority. The generated recipe remains the executable provisioning authority.

This scope does not generate device plans, infer configuration-copy behavior from README prose, or automatically add root, permission, force-stop, app-data, or configuration steps.

## App and recipe generator

### Supported sources

The first complete workflow supports:

1. local APK;
2. GitHub repository URL;
3. GitHub release URL; and
4. direct remote APK URL.

GitHub support uses the GitHub API and accepts only canonical GitHub repository and release identities. It does not scrape rendered HTML or implement a generic arbitrary-site crawler.

GitHub analysis collects bounded repository metadata, stable release metadata, and APK release assets. Drafts and prereleases are excluded by default. When multiple APK assets remain, the user must choose one.

### APK inspection

APK inspection is authoritative for Android-specific facts:

- package name;
- application label;
- version code and version name;
- minimum and target SDK;
- supported ABIs;
- launcher activities;
- requested permissions;
- debuggable status;
- split/base APK status; and
- signing-certificate SHA-256 fingerprint.

GitHub names, topics, descriptions, filenames, and release metadata may suggest values but may not substitute for manifest evidence.

The first implementation uses a separately configured user-supplied Android APK analysis tool. Preferred order is `apkanalyzer`, then `aapt2`. Neither tool is part of Android Platform-Tools, so this configuration is independent from ADB setup. EmuChef does not bundle Android SDK build tools.

APK analyzers run with direct argv, bounded output, and a timeout. APK inspection never installs the package. Split APKs are rejected for the starter single-APK recipe until an explicit split-install contract exists.

### Installation strategies

The wizard exposes three source-neutral recipe strategies when the selected source supports them.

#### Pinned release

This is the default for a selected GitHub release asset or direct HTTPS APK. The recipe declares a `remote_file` artifact with the exact asset URL, resolves it, and installs it. This mode is reproducible because the concrete artifact is fixed at authoring time.

#### Latest compatible release

This is available for public GitHub repository sources. The author selects one APK from the current release and EmuChef derives an editable filename regular expression by generalizing version-like segments while preserving variant, platform, and architecture text. The rule is previewed against the selected release and review is blocked unless it matches exactly one APK.

The generated recipe uses explicit `resolve_github_release` and `download_remote_file` steps. Runtime resolution excludes drafts, excludes prereleases unless the author enables them, orders releases deterministically by publication time and tag, and fails safely when the saved rule matches zero or multiple assets. Future releases may differ from the APK inspected during authoring.

#### User-provided APK

This is the default for a local APK and is available for any source. The recipe declares a required `file` input with role `apk` and installs that input. It never persists the developer's local absolute APK path.

### Generated recipe

The starter recipe is intentionally minimal:

- optional `resolve_artifacts` for a pinned remote source;
- optional `resolve_github_release` followed by `download_remote_file` for latest-compatible GitHub sources;
- `install_apk` constrained by `apk_install`;
- `package_installed` skip condition using the verified package name; and
- optional `launch_app` only when a launcher component was verified and the author explicitly enables launch-once generation.

The generator uses the existing recipe model and step specifications. It does not maintain a second recipe representation.

Current limitation: the runtime resolver and downloader are implemented, but executor-side APK package and signing-certificate reinspection is not yet available. Latest recipes therefore retain authoring-time identity evidence but do not yet enforce package or certificate identity immediately before install. That enforcement must be added before latest mode is considered supply-chain complete.

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
- an identical latest-release policy fingerprint composed of resolver, repository, asset pattern, and prerelease policy.

ID, path, and identical latest-policy conflicts are blocking. Matching package or repository data under a different ID is otherwise a warning requiring review. A pinned source and latest source for the same repository therefore warn through repository overlap without being treated as the same immutable artifact.

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

Add negotiated capabilities for:

- `analyzeAppSource`;
- `inspectApk`;
- `generateAppRecipeDraft`;
- `generateDeviceProfileDraft`; and
- `checkGeneratedCatalogCollisions`.

Existing `listAdbDevices` and `probeDevice` capabilities are reused.

Analysis and draft generation return structured data and perform no authored-data writes. Backend responses use stable error codes and redact credentials, exact serials, absolute paths, raw analyzer output, and unsafe network details from product-facing errors.

## Tauri and save ownership

Tauri owns:

- native local-APK selection;
- configured APK-analyzer paths;
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
- APK filenames, labels, repository fields, and analyzer output are untrusted input.
- External tools use direct argv and never a shell.
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

- configured APK analyzer;
- native APK picker;
- APK inspection;
- app-definition draft;
- user-provided-APK recipe draft;
- dual-document validation and save; and
- opening the generated recipe.

The implemented local workflow uses explicit BYO metadata:
`install_source.type` is `user_provided_apk`, its resolver is `none`,
`tracking_source.type` is `local_apk`, direct APK support is not required, and
BYO APK support is required. Verified APK facts are review evidence rather than
automatically persisted metadata.

Native selection accepts regular APK files up to 2 GiB and regular executable
files whose basename matches the explicitly selected `apkanalyzer` or `aapt2`
adapter. Analyzer processes use direct argument vectors, a 30-second timeout,
and a 4 MiB bound per output stream. Both adapters report certificate SHA-256
as missing because neither supported command surface exposes signing identity;
the missing fact is a deterministic warning rather than invented data.

The Config Editor enforces one active generator wizard. App-generator paths and
file identities remain behind session-scoped Tauri handles. Final dual-file
publication revalidates and rescans, uses synced temporary siblings and
create-new hard-link publication, removes the first publication when the
second fails and rollback is safe, then opens the recipe in the existing
document session.

### Phase 4: GitHub and remote APK sources

- repository and release analysis;
- stable asset filtering and selection;
- bounded temporary APK download;
- pinned-release recipe generation;
- direct remote APK mode; and
- network, timeout, and redaction tests.

The implemented workflow uses only the GitHub REST API. Drafts are excluded,
repository prereleases are opt-in, exact prereleases require confirmation, and
eligible assets are non-empty `.apk` files no larger than 2 GiB. Direct URLs
must use public HTTPS without credentials, fragments, or query parameters.
Metadata bodies are bounded to 2 MiB, redirects to five safe HTTPS hops,
connections to 10 seconds, and requests to 30 seconds. Downloads stream into a
session-owned temporary workspace and reuse the configured APK analyzer.

Pinned generation stores normalized source identity and emits a `remote_file`
artifact plus resolve/install steps. Authors may instead choose the existing
user-provided APK strategy, which preserves the Phase 3 `user_provided_apk`
and `local_apk` source shape without remote tracking fields. Authentication,
private repositories, arbitrary-site scraping, background refresh, and
split-package formats remain excluded.

### Phase 5: refinement

Potential later work includes OS-keychain GitHub credentials, dedicated app/profile editors, release-pattern testing, Obtainium import, source-update checks, aliases, and device-plan assistance.

## Verification

Rust tests cover typed parsing/emission, identifier normalization, regex escaping, Android ranges, evidence assignment, collision classification, generated recipe validity, URL parsing, release filtering, analyzer parsing, timeouts, and redaction.

Protocol tests cover capability negotiation, malformed requests, side-effect-free generation, stable errors, and absence of unsafe paths or serials.

Config Editor tests cover wizard transitions, cancellation, ambiguous APK selection, backend restart invalidation, collision blocking, dirty-document protection, save recovery, and opening the generated recipe.

Normal automated tests use local APK fixtures, fake GitHub HTTP endpoints, and fake ADB runners. They do not require public GitHub access, a real device, or installed Android SDK tools.
