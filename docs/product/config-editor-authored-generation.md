# Config Editor Authored Generation

## Status
Implemented current-state design. Typed authored foundations, device-profile generation, local APK generation, remote APK generation, App Definition authority, catalog-backed Recipe validation, planning, review, and execution are implemented. App Definitions own app identity, APK policy, permission sets, and app targets; Recipes reference those authorities and describe workflow. Private source access remains outside the supported workflow.

This document defines the Config Editor workflows for generating authored
content and editing App Definition documents:

- a starter app definition and installation recipe from a GitHub source, remote
  APK, or local APK;
- a starter device profile from a connected ADB device; and
- an existing catalog App Definition or a standalone App Definition YAML file.

Rust is the sole product runtime. All analysis, validation, canonical generation, collision detection, and sidecar protocol behavior belong to `crates/emuchef-rust-backend`. Tauri owns native paths, trusted filesystem writes, configured external tools, and exact ADB serials. React owns presentation and explicit user choices.

## Product boundaries

Generation creates reviewable authored drafts. It does not silently alter the catalog.

The generated recipe becomes a normal Recipe document and uses the existing
recipe editor, validation, undo/redo, reference indexing, and canonical YAML
path. App and device profile generation remain draft forms with YAML previews.
App Definitions also have a separate persistent typed document session for
direct editing; it does not change the generator draft or publication flow.

Generation operations are side-effect free until an explicit save. Network analysis may retrieve bounded metadata and a selected APK into a generator-owned temporary workspace, but it does not install APKs, execute repository content, modify a device, or publish catalog files.

App Definitions own intrinsic app identity and reusable policy: package identity, App Artifacts and their sources, permission sets, App Targets, launcher activity, and presentation metadata. Recipes own procedural ordering and reference the owning App Definition; they do not duplicate application authority.

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
App generation writes App Definition authority for app identity and APK policy. Every install Recipe step uses `app_ref` and resolves `params.app` from a Recipe `app_artifact` referencing an App Artifact of kind `apk` owned by that same App Definition.

#### Pinned release
A pinned APK source is declared on the App Definition App Artifact. The Recipe contains an `app_artifact` reference and a `resolve_artifacts` step; it does not duplicate the URL as a generic Recipe `remote_file`. A publisher-provided checksum belongs to the App Artifact, while locally calculated inspection hashes are review metadata only.

#### Latest compatible release
The App Definition owns the latest-release provider and selection policy. The Recipe still resolves the App Artifact through `resolve_artifacts`; it does not contain source-specific `resolve_remote_release` or `resolve_github_release` steps. `download_remote_file` remains supported for independent generic resources. Release matching must identify exactly one eligible APK.

#### User-provided APK
The App Definition declares a user-provided APK artifact. The Recipe binds its required APK input to that artifact with an `app_artifact` reference, resolves it, and installs its local path. The input path remains runtime data and is not persisted as application identity or source policy.

### Generated recipe
Generated install steps carry `app_ref`; their APK parameter references `artifacts.<id>.local_path` for the owning APK App Artifact. `package_installed` skip conditions have empty parameter maps and inherit package identity from the App Definition. Lifecycle and permission steps also use App Definition context rather than duplicated package, activity, or permission facts. Generic downloads remain independently available for non-app resources. Permission steps are emitted only when the author explicitly enables applicable permission automation.

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
app-op maps `MANAGE_EXTERNAL_STORAGE` to mode `allow`. Generated automation is
stored in the App Definition's `permission_sets.baseline.runtime` or
`permission_sets.elevated.app_ops` entries. The App Definition owns package
identity; permission actions do not repeat it and do not carry per-action
failure policy or `required` flags. A Recipe grants a selected set through an
App-bound `grant_permissions` step:

```yaml
type: grant_permissions
app_ref: example_app
params:
  permission_set: baseline
  policy:
    on_failure: warn
```

All permission sets require `shell_command`; the `elevated` set additionally
requires `root_shell` and executes through the reviewed root-authority path.
Individual actions may declare `android_api_min` and `android_api_max`. The
planner evaluates those bounds against the target device and marks actions
outside their supported API range `not_applicable` without suppressing other
eligible actions.

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

Every generated App Definition is a schema-v1 document under `authored/apps`
with:

- required `id`, `name`, and `package_id`;
- optional `description`, `category`, `launcher_activity`, and `metadata`,
  omitted canonically when absent; and
- ordered `artifacts` and `targets` mappings plus optional `permission_sets`,
  omitted canonically when empty.

`artifacts` entries are named and their kind is exactly `apk` or `file`; each
artifact may carry an optional display name and description. Sources use
exactly one strategy:

- `user_provided` for a file the user supplies at runtime, which is what local
  and user-provided APK generation produces;
- `direct_url` for a durable public HTTPS download; embedded credentials,
  fragments, and non-HTTPS schemes are rejected, query parameters are allowed,
  and the only optional extra is a lowercase 64-hex trusted publisher SHA-256.
  The generator stores an author-supplied trusted publisher checksum in
  lowercase and never copies the locally calculated inspection digest into it;
  and
- `latest_release` for a provider release described by `provider` (exactly
  `github`, `gitlab`, or `forgejo`), required service-origin `base_url`,
  `repository`, optional `asset_pattern`, and a required `prerelease` policy.
  GitHub and Forgejo repositories are exactly `owner/repository`; GitLab
  accepts nested namespaces such as `group/subgroup/project`.
  The runtime release resolver resolves GitHub and GitLab releases from their
  official service origin, so generated drafts accept only
  `https://github.com` for `github` and `https://gitlab.com` for `gitlab`
  (`latest_release_base_url_unsupported` otherwise) until resolution honors
  authored origins; `forgejo` resolves from the authored origin.

An absent `asset_pattern` selects candidate assets by artifact kind alone, so
an `apk` artifact accepts eligible APK assets whatever their file names. The
resolved release must still contain exactly one eligible asset: zero eligible
assets is a no-match failure and multiple eligible assets is an ambiguity
failure. `invert_asset_pattern` is legal only alongside a pattern and is
omitted canonically when false.
The App Artifact source policy retains `invert_asset_pattern` through planning
and execution. Release selection keeps pattern matches when the flag is false
and non-matches when it is true. The generated Recipe selects the App Artifact;
it does not duplicate or reinterpret the source filter.

Verified permission selections populate the App Definition permission sets.
Rust partitions each inspection-verified selection by its root requirement:
actions that run without root form the `baseline` set, actions that require
root form the `elevated` set, and an empty partition is omitted. Generated
Recipes reference the applicable named set from `grant_permissions`; permission
definitions remain App-owned. A present but empty set is invalid. Editing
permission sets outside the structured generator selections remains deferred.

`launcher_activity` is recorded only when inspection verified the exact
launcher component and the author enabled launch-once generation, converting
the inspected Android component form (`package/.Activity`) to the equivalent
fully qualified class name. The current native inspection reports no launcher
activities, so generated App Definitions normally omit the field. `targets`
record literal semantic destinations (`app_data`, `external_app_data`,
`shared_storage`, or `absolute_device_path`) with file or directory kind.

`metadata` stays non-authoritative and JSON-compatible, and direct metadata
keys may not shadow canonical top-level schema names. APK inspection evidence
is transient draft and review evidence: it appears in generator review, and it
is never written into the saved App Definition, including
`metadata.apk_inspection`.

Generators emit canonical schema-v1 Recipes. APK install parameters reference
the local path of a Recipe App Artifact, and the install step carries the same
`app_ref` as that App Artifact. Package, launcher, source, checksum, and
permission identity remain owned by the App Definition. Recipe App Artifact
references identify which App Artifact to resolve; changing or removing its
source changes the App Definition instead of duplicating policy in Recipe
parameters. A generated source must pair with one unambiguous App Artifact of
the selected kind.
Several artifacts that share the reviewed source strategy and none of them
under the generated artifact id are equally blocking
(`remote_source_artifact_ambiguous`), because generation cannot tell which
artifact the reviewed source belongs to. Only an `apk` artifact can represent
the reviewed source, because the generated Recipe installs an APK; retyping
the reviewed artifact as a generic `file` leaves no pairing artifact and
blocks for the same reason. Remote user-provided generation applies
the same presence rule to its user-provided APK artifact, which blocks as
missing or ambiguous. The filename pattern and prerelease toggle in the
source step mirror the reviewed artifact row while a draft exists, so the
release preview, the review gate, and the save gate all follow the same policy
the App Definition and Recipe are generated from. Only a latest-release source
carries that policy, so a pinned artifact never resets the two controls. A
source-step edit rewrites
only the artifact native generation pairs with the reviewed source: the
artifact the backend proposes for that source, named `apk`, while it is still
an APK whose source strategy matches the reviewed source, even after the
author retargets its URL or repository and even when it records another
service origin, otherwise a single matching artifact, and no row when the
pairing is unresolvable. A prerelease toggle that keeps the
selected asset eligible keeps the rows the pairing is derived from. Starting a
new remote download
clears the source-step publisher checksum, because the freshly downloaded
artifact has no author-supplied publisher checksum. Only the paired artifact
receives source-step edits, so every other authored row keeps its own policy.
Release analysis retained from an earlier source is presented only while it
still describes the repository the paired artifact resolves from; after the
author retargets the source, the step
stops using that match preview, the review and save gates do not block on it,
and Rust reports a non-blocking `latest_release_analysis_stale` warning so the
author can analyze the edited source.

Local generation keeps the same coherence rule in the other direction: the
generated Recipe declares a user-provided APK input, so structured mapping
edits must leave exactly one user-provided APK artifact in the App Definition.
Removing it, retyping it as a generic `file`, or replacing its source with a
remote strategy is blocking (`local_artifact_missing`), and leaving several
user-provided APK artifacts without the generated one is blocking
(`local_artifact_ambiguous`). A renamed artifact that stays user-provided and
is the only candidate still pairs with the recipe input.

### Evidence

Each proposed value carries one of these confidence states:

- `verified`: supplied by APK manifest or exact source metadata;
- `derived`: deterministically transformed from verified data;
- `suggested`: heuristic authoring recommendation; or
- `missing`: no value is proposed.

The draft response includes provenance and warnings. Final YAML contains authored values, not evidence wrappers.

### Collision detection

Before saving, Rust scans the selected authored root for:

- duplicate App Definition ID;
- duplicate App Definition `package_id`;
- duplicate recipe ID;
- duplicate destination path;
- an App Definition whose latest-release policy fingerprint (provider, base
  URL, repository, asset pattern, inversion, and prerelease policy) matches
  another App Definition;
- an App Definition source repository or direct download address also used by
  another App Definition; and
- App/artifact reference-integrity conflicts.

App Definition ID, App Definition `package_id`, Recipe ID, destination path,
and reference-integrity conflicts are blocking. Source-policy overlaps are
advisory. Recipe-derived APK package/checksum/permission fingerprints are not
collision authority. Rust checks the complete generated Recipe returned by
the trusted generation request; React cannot supply authored paths or
collision findings. Collision requests without a complete Recipe retain the
same identity, destination, and reference-integrity checks.

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

Rust owns typed schema-v1 models for app definitions and device profiles. These
models provide:

- structural parsing;
- canonical YAML emission;
- stable validation;
- regex and Android-range validation for device profiles;
- package, artifact-source, permission-set, target, and launcher validation for
  app definitions; and
- common authority used by generator output, save validation, catalog loading,
  and future dedicated editors.

Generated YAML is never treated as an unvalidated `serde_json::Value` blob.

Schema-v1 parsing rejects unknown fields in fixed top-level and nested
structures and rejects retired App Definition fields instead of migrating
them. Extensibility is limited to `metadata`, which retains nested
JSON-compatible values and insertion order without silently discarding data.
Authored IDs use lowercase alphanumeric segments separated by `.`, `_`, or
`-`. App Definition `artifacts` and `targets` registries reject a repeated id
while parsing, because YAML would otherwise keep only the last definition and
canonical emission would drop the earlier authored policy without a
diagnostic.

Canonical YAML emits fixed fields in schema order. Optional scalar values,
optional Android range bounds, and empty optional App Definition sections are
omitted when absent; device-profile collection fields are emitted explicitly
even when empty, and ordered mappings retain authored order. Re-emitting
canonical YAML is byte-stable.

## Sidecar protocol

Negotiated capabilities include:

- `openAppDefinition`;
- `applyAppDefinitionCommand`;
- `saveAppDefinition`;
- `saveAppDefinitionAs`;
- `saveDocument`;
- `saveDocumentAs`;
- `analyzeAppSource`;
- `inspectApk`;
- `generateAppRecipeDraft`;
- `generateDeviceProfileDraft`; and
- `checkGeneratedCatalogCollisions`.

Existing `listAdbDevices` and `probeDevice` capabilities are reused.
`saveDocument` and `saveDocumentAs` dispatch by document type alongside the
App Definition-specific save operations.

Analysis and draft generation return structured data and perform no
authored-data writes. Backend responses use stable error codes and redact
credentials, exact serials, absolute paths, parser diagnostics, and unsafe
network details from product-facing errors.

## Tauri and save ownership

Tauri owns:

- native App Definition file selection and Save As destinations;
- native local-APK selection;
- session-scoped native APK handles and file identities;
- exact ADB serials and device handles;
- native save destinations;
- final collision revalidation; and
- trusted writes.

The Rust sidecar owns typed App Definition drafts, diagnostics, canonical YAML,
history, dependency admission, collision checks, stale-state detection, and
single-file publication. Tauri supplies native paths and enforces trusted
filesystem boundaries. The backend rechecks relevant disk inputs immediately
before publication and rejects changes detected there. Atomic rename does not
provide compare-and-swap protection against an external write racing after that
check. App Definition publication is a single-file operation; the generator's
existing app-and-recipe publication flow remains separate.

Saving an app and recipe is one logical operation. Both drafts are validated before publication. Temporary files are written and synced before either final path is published. Existing files are never overwritten without explicit approval. If final publication partially fails, Tauri removes any newly published counterpart when safe and reports the incomplete outcome.

After a successful app-and-recipe save, the generated recipe opens through the existing recipe document session.

Device-profile roots and device serials use handles scoped to one ephemeral
generator session. React receives no native root path. A profile save
revalidates and rescans immediately, rejects incomplete collision scans and
existing destinations, writes and syncs a temporary sibling, and publishes with
atomic create-new/no-clobber semantics.

## Config Editor workflow

### App Definition documents

The selected catalog and the catalog that owns an App document are distinct.
The App document DTO always includes `owningAuthoredRoot`: the physical owner's
canonical path for a catalog App, or `null` for a standalone document.
`authoredRoot` is the currently selected catalog. Changing the selection does not
move ownership, unlock identity, redirect Save, or change which owner-catalog
Recipes participate in Save admission. Successful canonical Save As publication
establishes ownership for a standalone document.

The App-only `editRestrictions.canSaveAs` flag requires a standalone document
and a canonical, existing selected root directory. Existing real-directory
restrictions reject roots that are files and symlinked apps directories. An
unavailable or invalid root requires catalog selection before the native Save
As dialog can offer its canonical destination. Strict publication admission
still validates identity, dependencies, destination safety, and no-clobber
creation.

Standalone App Save As defaults to the selected catalog's
`apps/<app-id>.yaml` path. The native file dialog chooses a destination without
selecting or changing the catalog root. An author must select a catalog before
publication, and the destination must belong to that selected catalog. A catalog
App cannot use Save As to move or clone its identity.

Authors can open an App Definition from the Config Editor's native file actions
and edit it as a persistent typed document. The editor presents Overview,
Artifacts, Permissions, Targets, and Metadata sections, with canonical YAML and
diagnostics alongside the form. Recipe editing and app/device profile
generation remain separate workflows. Successfully opening an App Definition
exits the user-configuration view.

The sidecar accepts only bounded, tagged App Definition commands for defined
fields and named collection operations. App, artifact, and target identifiers
are not renameable through local edits. Catalog-backed documents keep their
opened ID, package identity, canonical file path, and owner root for admission.
Ownership is inferred from the physical `apps/` location independently of the
selected authored-root context; changing that context does not change the
document's owner root. Changing an identity that
would require updating references is not supported by this editor. A different
package identity requires a separate App Definition.
Opening rejects symlinked `apps/` directories in either the selected authored
root or the physical owner root.
Metadata remains descriptive, carries no authority, and rejects reserved
top-level keys and duplicate metadata keys.

A structurally parseable schema-v1 App Definition opens even when semantic
diagnostics are present. An incompatible kind or unsupported version is rejected
with structured field diagnostics before a session is created; headers are not
automatically converted. Malformed input cannot open as an App Definition
document. Authors can retain incomplete form input locally and submit complete
field or collection changes incrementally; a correction can succeed while
unrelated semantic diagnostics remain. Malformed command payloads and invalid
mutations to the field being changed are rejected without altering the draft or
current diagnostics or undo/redo history; candidate rejection diagnostics are
reported separately. Save is admitted only after the complete App Definition
and its directly affected persisted Recipes pass validation in full. Admission
combines shared local validation without device context, shared App authority
validation against the candidate and relevant other Apps in the snapshot, and
bounded static planning. Existing Ref and StepSpec rules for file outputs,
multiple inputs, copy provenance, resolver ancestry, and required permission
policy remain enforced. Errors anywhere in an affected Recipe block saving,
including errors outside the edited app context. A device API level is not
required; limited-context warnings are nonblocking, while static permission
policy is still validated.

Toolbar and native-menu Save and Save As actions follow the active document's
effective save availability, including backend edit restrictions. Pending App
form input counts as dirty work and disables both actions until it is applied or
explicitly discarded.
Undo/redo preserves pending local form fields while refreshing other fields
from the current typed document. If undo removes an artifact or target with
pending input, the form clears its backend identity and keeps the input
available to add as a new entry.

Dependency discovery covers typed App Artifact references, step app context,
permission-set selection, launcher use, and App Target references in step
parameters and conditions. If the backend cannot reliably classify every
Recipe because a directory cannot be read or a Recipe cannot be parsed, Save
fails with an actionable diagnostic. Admission checks only relevant App ID and
package conflicts; known errors in unrelated catalog documents do not block a
valid save, and source overlap remains advisory.

The backend rechecks the opened App Definition, Recipe inventory and affected
Recipe files, and competing App identity files immediately before publication.
Changes detected by that recheck produce a stale-state diagnostic. Publication
replaces one file atomically or creates the canonical Save As destination
without clobbering an existing file. Replacement saves preserve the existing
Unix file mode; a detected mode change causes stale-state rejection.
Filesystem publication is not an atomic compare-and-swap operation, so an
external write racing after the final recheck
is outside the stale-state guarantee. Rejected saves preserve disk contents and
the editable draft. Save failures refresh only the active document's diagnostics
panel with file and object locations without changing document content or
history. This editor does not rename identities or rewrite
cross-document references.
App Definition diagnostics use normalized object locations across document
operations.

Saving a standalone document updates its opened external file. Save As requires
a selected authored root and publishes it once to `apps/<id>.yaml` there; the
file-dialog destination cannot infer or change that authority. The session adopts
that canonical identity only after publication succeeds and removes undo/redo
history entries incompatible with the newly locked identity. Catalog-backed
documents cannot use Save As to move or rename their identity.
Other-root destinations, dot-segment paths, and symlink escapes are rejected.
Canonical filesystem aliases such as `/var` and `/private/var` are accepted.

Document-kind-aware actions route Save, undo/redo, validation, YAML preview, and
dirty-state prompts to the active document. Dirty work is protected when
switching document kinds, opening a generator, closing a window, or recovering
from a sidecar restart. A canceled or failed open preserves the active session.
Parseable but semantically invalid App Definitions remain available for repair;
Save remains blocked until admission succeeds.

Top-level actions:

```text
File
  Open App Definition...
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

The implemented local workflow generates an App Definition whose `apk`
artifact uses the `user_provided` source strategy. Native APK inspection facts
are transient draft and review evidence and are never written into the saved
App Definition; local flows record no permission sets. The recipe keeps its
required user-provided APK input.

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

Pinned generation records the source on the App Definition APK App Artifact
and emits a Recipe App Artifact reference plus `resolve_artifacts` and install
steps. Publisher checksums remain on the App Artifact and are never copied
into Recipe install parameters. The user-provided strategy binds a required
Recipe input to a `user_provided` App Artifact. Authentication, private
repositories, arbitrary-site scraping, background refresh, and split-package
formats remain excluded.

### Phase 5: GitHub release-pattern testing

GitHub repository sources using the latest-compatible strategy expose an
immediate filename-pattern preview after source analysis. GitHub analysis
requests the most 30 releases. The trusted session retains every non-draft
release in provider response order, including prereleases and releases with no
eligible APK assets. The Include prereleases selection filters the retained set
locally and does not make another network request.

The preview applies the author-entered regular expression to each release's
eligible APK filenames using substring-search semantics unless the expression
itself supplies anchors. The pattern is optional: when it is blank, the preview
treats every eligible APK filename as a candidate and reports the same
outcomes. It reports `unique_match`, `no_match`, or `multiple_matches` for each
eligible analyzed release and sorts displayed matching filenames
deterministically. Summary counts cover the complete eligible retained set.
The UI displays at most the first 10 rows in provider response order.

For the check, the current release is the first retained release after draft
exclusion and prerelease filtering. This follows provider response order; the
workflow does not claim that GitHub guarantees chronological ordering and does
not reorder releases by tag, semantic version, filename, or parsed timestamp.
The current release must contain exactly one match, which for a blank pattern
means exactly one eligible APK asset. No trusted analysis, empty analysis, no
releases after prerelease filtering, zero current-release matches, multiple
current-release matches, and invalid Rust regex syntax are blocking. Older
zero-match and multiple-match results remain visible warnings when the current
release has one match.

The trusted release snapshot belongs to the analyzed source. When the reviewed
artifact retargets its provider, service origin, or repository, that evidence
no longer describes the edited source, so pattern checks are reported as a
non-blocking `latest_release_analysis_stale` warning instead of being judged
against releases from a different repository.

The browser preview uses JavaScript regular-expression behavior for immediate
feedback and is not final validation. Tauri ignores browser-computed ordering,
counts, outcomes, and release contents. It constructs a minimal ordered
snapshot from session-owned analysis containing only release tags, flags, and
eligible APK filenames. Rust evaluates the raw pattern again with the
production `regex` engine before draft generation and saving; a blank pattern
is omitted from the saved App Definition rather than stored as an empty string,
and a pattern accepted by JavaScript but rejected by Rust is blocking. Pinned
remote assets, direct APK sources, and user-provided APK strategies do not
require or consume release-pattern results and retain their existing generated
recipe shapes.

### Later refinement

The canonical remaining Config Editor backlog is maintained in
[`product-roadmap.md`](product-roadmap.md) under **Part II — Config Editor
Roadmap**. That track currently includes OS-keychain GitHub credentials,
dedicated app-definition and device-profile editors, Obtainium import,
source-update checks, alias management, device-plan assistance, and deferred
extended device capability checks.

These are Config Editor requirements. They do not change EmuChef proper Phase 5
status unless the canonical roadmap explicitly assigns a shared dependency or
cross-product acceptance criterion.

## Verification

Rust tests cover typed parsing/emission, identifier normalization, regex
escaping, Android ranges, evidence assignment, collision classification,
generated recipe validity, native manifest parsing, permission classification,
install-time package/checksum enforcement, URL parsing, release filtering,
timeouts, and redaction.

Protocol tests cover capability negotiation, malformed requests, side-effect-free generation, stable errors, and absence of unsafe paths or serials.

Config Editor tests cover wizard transitions, cancellation, ambiguous APK selection, backend restart invalidation, collision blocking, dirty-document protection, save recovery, and opening the generated recipe.

Normal automated tests use local APK fixtures, fake GitHub HTTP endpoints, and fake ADB runners. They do not require public GitHub access, a real device, or installed Android SDK tools.
