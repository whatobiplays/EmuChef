# App Definition and Recipe Authority Model

**Status:** Accepted final v1 target architecture. The completed contract has no transitional legacy authority forms or fallback semantics. Implementation is proceeding incrementally, so some transitional forms remain until the final authored-data cutover; this document does not claim that every described capability is available today.

The App Definition document editor and App Generator components already exist. Their complete authority-aware workflows, planner-derived Recipe availability ([#19](https://github.com/whatobiplays/EmuChef/issues/19)), catalog-backed Recipe controls ([#21](https://github.com/whatobiplays/EmuChef/issues/21)), atomic catalog refactors ([#22](https://github.com/whatobiplays/EmuChef/issues/22)), and final authored-data/contract cutover ([#23](https://github.com/whatobiplays/EmuChef/issues/23)) remain migration work. Generator workflow completion is tracked in [#20](https://github.com/whatobiplays/EmuChef/issues/20).

## 1. Authority model

EmuChef has four distinct authored/runtime authorities:

1. **App Definition** — application authority. It owns facts and policy that are intrinsically true about one Android application.
2. **Recipe** — procedural authority. It owns executable workflow, ordering, dependencies, toggles, and failure policy.
3. **Device Plan** — orchestration authority. It selects and composes Recipes for a supported device setup.
4. **Execution Plan** — resolved execution authority. It is an immutable, catalog-independent snapshot consumed by both review and execution.

Core invariant:

> If a value is intrinsically true about an app, a Recipe does not duplicate it.

An Execution Plan may retain immutable denormalized IDs and presentation snapshots for provenance, UI, reporting, and error messages, but neither review nor execution may reopen authored App Definitions, Recipes, or Device Plans to determine behavior.

"Fully resolved" does not mean all network results have already been fetched. A late-bound release policy may remain in the Execution Plan, provided it is self-contained and requires no authored-catalog dereference.

## 2. Direct schema-v1 migration

The accepted final v1 contract changes App Definition v1 and Recipe v1 directly. After the final cutover, there are no legacy readers, fallback semantics, mixed old/new forms, or compatibility aliases. All app-aware authored data and the generator, editor, validation, planner, review, and documentation surfaces conform to that contract at cutover.

Implementation is proceeding in incremental migration slices. Transitional forms may remain before the final cutover, which Issue #23 tracks. The old forms are rejected when that cutover is complete.

## 3. App Definition v1

An App Definition owns exactly one Android package identity.

A canonical App Definition has this conceptual shape:

```yaml
schema_version: 1
kind: app_definition

id: retroarch
name: RetroArch
description: Multi-system emulator frontend.
category: emulator

package_id: com.retroarch.aarch64

artifacts:
  apk:
    kind: apk
    name: RetroArch APK
    source:
      strategy: latest_release
      provider: github
      base_url: https://github.com
      repository: owner/repository
      asset_pattern: '...'
      invert_asset_pattern: false
      prerelease: false

permission_sets:
  baseline:
    runtime:
      - permission: android.permission.POST_NOTIFICATIONS
        android_api_min: 33

targets:
  system_directory:
    kind: directory
    location: shared_storage
    path: RetroArch/system

launcher_activity: com.example.MainActivity

metadata:
  homepage: https://example.com/
```

Optional absent sections are omitted rather than emitted as empty mappings.

### 3.1 Identity

- `id` is authored catalog identity.
- `name` is display identity and may change freely.
- `package_id` is the single Android package identity owned by the App Definition.
- Package IDs are globally unique across App Definitions.
- A different package ID requires a different App Definition.
- Package aliases do not exist.
- `category` is optional free-form product metadata.
- `metadata` is opaque, non-authoritative JSON-compatible data.
- Canonical top-level schema names are reserved and invalid as direct keys inside `metadata`.
- Persisted `metadata.apk_inspection` is removed. APK inspection remains transient generator/review evidence.

An existing App Definition generator flow may never change package identity as part of "add artifact/recipe". An inspected package mismatch is a hard blocker and requires creation of a different App Definition.

### 3.2 App Artifacts

An **App Artifact** is a named app-owned resource. Independently selectable builds are separate named artifacts; there is no nested "variant" contract.

Example:

```yaml
artifacts:
  apk_android_11: ...
  apk_android_13: ...
  apk_android_15: ...
```

A Recipe explicitly chooses one named artifact. The planner does not select a compatible build variant automatically.

Initial artifact kinds:

- `apk`
- `file`

`apk` adds APK-specific semantics:
- the materialized file must be an APK;
- its manifest package must equal the App Definition `package_id`;
- it is eligible for `install_apk`.

`file` is generic. Archives remain `file`; extraction behavior belongs to procedural steps.

Artifacts may have optional `name` and `description`. There is no artifact-level arbitrary metadata map in v1.

### 3.3 App Artifact source strategies

Exactly three v1 strategies exist.

#### User-provided

```yaml
source:
  strategy: user_provided
```

The Recipe artifact declaration binds the runtime file input.

#### Direct URL

```yaml
source:
  strategy: direct_url
  url: https://example.com/file.apk
  sha256: 012345...
```

Rules:
- HTTPS only in v1.
- Credentials and URL fragments are rejected.
- Query parameters are allowed.
- Ephemeral authenticated/signed URLs are not durable public catalog sources.
- Authentication and credential references are deferred to v2.
- `sha256` is optional, source-level, and canonicalized as lowercase 64-character hexadecimal.
- Static trusted SHA-256 is valid only for `direct_url`.

#### Latest release

```yaml
source:
  strategy: latest_release
  provider: github
  base_url: https://github.com
  repository: owner/repository
  asset_pattern: 'arm64.*\.apk$'
  invert_asset_pattern: false
  prerelease: false
```

Supported providers:
- `github`
- `gitlab`
- `forgejo`

`base_url` is mandatory and represents the canonical service origin, not a provider API endpoint. Provider code derives the correct API surface.

Authentication is unsupported in v1.

Asset resolution:
1. select the latest eligible release according to provider policy and `prerelease`;
2. apply artifact-kind filtering;
3. apply `asset_pattern` to the provider asset filename only;
4. retain matches when `invert_asset_pattern: false`, or retain non-matches when true;
5. require exactly one remaining asset.

Zero matches and multiple matches fail resolution.

For `kind: apk`, the remaining asset must also satisfy APK semantics. For `kind: file`, there is no APK-extension restriction.

The strategy is intentionally late-bound. Review approves the resolver policy, not a specific release discovered during planning.

## 4. App Targets

An **App Target** is a named app-owned semantic device destination.

Shape:

```yaml
targets:
  main_config:
    kind: file
    location: app_data
    path: files/config.ini
```

There is no `role` field.

Kinds:
- `file`
- `directory`

Locations:
- `app_data`
- `external_app_data`
- `shared_storage`
- `absolute_device_path`

v1 targets Android user 0.

Resolution:
- `app_data` → `/data/user/0/<package_id>/<relative path>`
- `external_app_data` → `/storage/emulated/0/Android/data/<package_id>/<relative path>`
- `shared_storage` → `/storage/emulated/0/<relative path>`
- `absolute_device_path` → exact literal absolute authored path

Relative semantic paths:
- must be relative;
- reject `.` and `..` segments;
- reject repeated separators;
- reject environment variables, `~`, interpolation, placeholders, and semantic rewriting.

Absolute-device paths:
- must be absolute;
- reject `.` and `..` segments and repeated separators;
- remain literal.

Derived capabilities:
- `app_data` → `app_data_write`
- `external_app_data` → `app_data_write`
- `shared_storage` → `shared_storage_write`
- `absolute_device_path` → no inferred capability

Target `kind` is operational:
- a directory/path-list source requires a directory target;
- a single-file source may target a directory or exact file target;
- `copy_policy: sync` is valid only for directory destinations.

## 5. Launcher identity

An App Definition may contain one optional canonical `launcher_activity`.

It is stored as a fully qualified activity class name, not a combined package/activity component and not relative shorthand.

`launch_app` is invalid for an App Definition without a launcher activity.

Named launch targets are deferred until a demonstrated need exists.

## 6. Permission sets

Permission automation is optional. When specified, it must be valid.

Exactly two standardized sets exist:
- `baseline` — normal/non-root permission or app-op automation;
- `elevated` — actions that require root.

Either set may be absent. Empty sets are invalid and should be omitted.

Selecting:
- `baseline` intrinsically derives `shell_command`;
- `elevated` intrinsically derives `shell_command + root_shell`.

The Recipe does not duplicate those capability constraints.

Runtime-permission action:

```yaml
- permission: android.permission.POST_NOTIFICATIONS
  android_api_min: 33
  android_api_max: 35
```

App-op action:

```yaml
- op: MANAGE_EXTERNAL_STORAGE
  mode: allow
  android_api_min: 30
```

Rules:
- `android_api_min/max` are optional per action;
- omitted bounds mean all API levels;
- no `required`;
- no `requires_root`;
- no package ID;
- no per-action failure policy;
- app-op mode is explicit, with only `allow` accepted in v1;
- duplicate logical actions are rejected within a set and across baseline/elevated.

The planner resolves API applicability for the target device. If zero actions in a selected set are applicable, the Execution Plan retains the permission step with a not-applicable disposition; execution reports it skipped without warning or failure.

Recipe-owned failure policy is only:

```yaml
policy:
  on_failure: warn | fail
```

There is no `require_all`.

Execution attempts every applicable independent action, then aggregates:
- `warn` → succeeded with warning when any applicable action fails;
- `fail` → failed when any applicable action fails;
- inapplicable actions do not count as failures.

## 7. Recipe app context

App association belongs to the relevant step, never to the Recipe as a whole.

Canonical step form:

```yaml
- id: ...
  type: copy_files
  app_ref: retroarch
```

`app_ref` is a first-class Step field, not a param and not a generalized runtime reference.

Every StepSpec declares:

- `none`
- `optional`
- `required`

Initial matrix:

| Step | App context |
| --- | --- |
| `install_apk` | required |
| `grant_permissions` | required |
| `launch_app` | required |
| `force_stop_app` | required |
| `copy_files` | optional |
| `resolve_artifacts` | none |
| `extract_artifacts` | none |
| `extract_archive` | none |
| `wait` | none |
| generic low-level download operations retained in v1 | none |

Validation:
- required + missing `app_ref` → error;
- unknown `app_ref` → error;
- none + present `app_ref` → error;
- optional app context may be present only when the step actually consumes app-owned context;
- all app-owned values used by one step must resolve to the same App Definition.

### 7.1 App-aware conditions and verification

Conditions never declare their own `app_ref`. They inherit enclosing step context.

`package_installed` becomes parameterless and requires enclosing app context.

Typed `app_target` values may be used by compatible condition parameters. An app-aware condition on a step without app context is invalid.

## 8. Recipe App Artifact declarations

Recipes make app-owned artifacts explicit through normal recipe artifact plumbing:

```yaml
artifacts:
  installer:
    type: app_artifact
    app_ref: retroarch
    artifact: apk
```

For `user_provided`:

```yaml
artifacts:
  installer:
    type: app_artifact
    app_ref: retroarch
    artifact: apk
    input:
      ref: inputs.retroarch_apk
```

Rules:
- `input` is required for `user_provided`;
- `input` is forbidden for `direct_url` and `latest_release`;
- the input must be one compatible `file` input;
- the input does not need `role: apk`;
- artifact kind/package semantics supply APK validation.

Downstream steps use normal runtime artifact refs such as:

```yaml
params:
  app:
    ref: artifacts.installer.local_path
```

App Artifacts participate normally in artifact groups, `resolve_artifacts`, and extraction.

Any app-aware consumer must have coherent app provenance with the artifact/target it consumes.

## 9. Generic Recipe artifacts and inputs

Generic non-app resources remain ordinary Recipe artifacts.

```yaml
artifacts:
  helper_archive:
    type: remote_file
    url: https://example.com/helper.zip
    cache: default
```

Generic `remote_file` remains supported.

Generic Recipe input `role` remains optional product/presentation metadata for now. It is no longer execution authority for APK semantics.

Source-specific authored release resolver steps are removed:
- `resolve_remote_release`
- `resolve_github_release`

`download_remote_file` remains as a low-level generic operation in v1.

There is no first-class generic Recipe Artifact `latest_release` strategy in v1. If later required, it should be added as a typed generic artifact source rather than reintroducing provider-specific procedural release steps.

## 10. App-aware StepSpec simplification

### install_apk

`install_apk`:
- requires `app_ref`;
- may consume only a Recipe `app_artifact` whose App Artifact kind is `apk`;
- authored Recipe parameters no longer include `expected_package_name` or `expected_sha256`;
- package identity and checksum policy derive from the App Definition/App Artifact;
- APK manifest package verification occurs before ADB installation.

### launch_app

`launch_app`:
- requires `app_ref`;
- has no authored package/activity identity params;
- planner resolves package ID and canonical launcher activity.

### force_stop_app

`force_stop_app`:
- requires `app_ref`;
- has no authored package param;
- planner resolves package ID.

### grant_permissions

Canonical authored shape:

```yaml
- type: grant_permissions
  app_ref: retroarch
  params:
    permission_set: baseline
    policy:
      on_failure: warn
```

`permission_set` is required. Runtime/app-op action arrays no longer live in the Recipe.

## 11. Artifact materialization

`resolve_artifacts` is the uniform materialization boundary.

Conceptually:

```text
direct_url     → download/cache → integrity validation → local_path
latest_release → resolve release → download/cache → validation → local_path
user_provided  → validate/bind supplied input → local_path
remote_file    → existing generic recipe artifact resolution
```

App source strategy does not leak into Recipe control flow.

For `direct_url`, SHA-256 is enforced during artifact resolution before the resolved artifact is published to downstream consumers.

For APK App Artifacts, package verification must complete before device mutation.

Any runtime artifact-field consumer must have a dependency ancestor that resolves that artifact, directly or through an artifact group. Missing materialization dependency is authored/planner validation failure, not an execution-time surprise.

Recipe-owned cache policy remains on the Recipe artifact declaration:
- `default`
- `none`

App Definitions do not own cache policy.

## 12. Execution Plan

The planner normalizes authored artifacts into a self-contained runtime artifact tagged union, conceptually:

```yaml
source:
  type: local_input
  path: ...
```

```yaml
source:
  type: remote_url
  url: ...
  sha256: ...
```

```yaml
source:
  type: remote_release
  provider: github
  base_url: ...
  repository: ...
  asset_pattern: ...
  invert_asset_pattern: false
  prerelease: false
```

These are runtime policies, not authored App Definition references.

Resolved plan provenance uses IDs rather than unresolved `*_ref` names:
- `device_plan_id`
- `device_profile_id`
- selected/expanded Recipe IDs
- step `recipe_id`
- optional step `app_id`

The plan retains immutable catalog identity/version/content digest provenance.

### 12.1 Presentation snapshots

Review must not reopen authored documents after planning.

The Execution Plan therefore includes minimal immutable presentation snapshots:

- Device Plan: ID, name, description;
- Recipes: ID, name, description;
- Apps: ID, name, description, optional category, package ID;
- resolved inputs: identity/type/value plus label, description, required/sensitive state, and selected enum presentation where relevant.

The plan does not copy entire authored documents.

## 13. Late-bound release review and reporting

A `latest_release` plan may resolve to a newer eligible release between review and execution.

Review must describe the approved policy truthfully, e.g. "Download the latest eligible release from GitHub," rather than pretending a specific version is already fixed.

The terminal execution report records the actual materialization result, including when applicable:

- artifact ID;
- release tag;
- asset filename;
- provider published-at metadata;
- final download URL;
- calculated SHA-256;
- cache hit/miss.

Direct-URL and user-provided artifacts also record calculated SHA-256 so the report identifies the exact bytes used.

The durable cache metadata may remain more privacy-minimal than the execution report; report provenance and cache indexing are separate concerns.

## 14. Capability gating and product availability

Derived app capabilities participate in the same existing step-gating semantics as authored step capability constraints.

A missing capability may omit the affected step rather than making the whole plan invalid when the remaining selected-step graph is valid.

Accordingly, product configuration must not determine Recipe availability by unioning every step capability.

The old `requiredCapabilities` union is removed from product Recipe projection.

In the accepted target, `available` and `unavailableCapabilities` are planner-derived:
- an optional/gated elevated permission step on a non-root device does not make the Recipe unavailable;
- a genuinely root-only Recipe may be unavailable and report `root_shell`.

## 15. Product catalog projection

The target product catalog exposes App Definitions as first-class safe entities, including at least:

- ID;
- name;
- description;
- category;
- package ID.

Recipe descriptors may expose the App Definition IDs they operate on, derived from step/artifact usage.

`contentRequirements` are derived from effective artifact/input behavior:
- user-provided APK/file → user file requirement;
- direct URL/latest release/generic remote artifact → network download;
- generic BIOS/ROM roles may continue to support current presentation until redesigned separately.

## 16. Device Plan responsibility for build-specific Recipes

Build/device compatibility is not an App Artifact variant-selection problem.

For cases such as separate ARMSX2 Android 11/13/15 APKs:
- the App Definition owns separate named artifacts;
- separate Recipes explicitly choose the desired artifact;
- Device Plans select the correct Recipe for the supported setup.

Recipes do not gain an automatic Android compatibility selector solely to choose among those builds.

## 17. Config Editor

The Config Editor has an App Definition document editor. Completing the authority-aware Recipe editing workflow remains target work; Issue #21 tracks catalog-backed controls for app-aware Recipe steps.

The document system shares lifecycle mechanics while preserving typed domain models:

```text
Document session lifecycle
├── RecipeDocument
├── AppDefinitionDocument
└── future typed document kinds
```

`AppDefinitionDocument` owns:
- typed app-edit commands;
- undo/redo;
- validation;
- canonical YAML;
- dirty state;
- reference diagnostics.

In the completed target workflow, Recipe app selectors are catalog-backed rather than free text.

Once an app is selected:
- `install_apk` choices filter to coherent APK App Artifacts;
- `grant_permissions` offers only defined permission sets;
- app-target selectors filter by target kind;
- `launch_app` can immediately report missing launcher identity.

## 18. Generator

The Config Editor includes an App Generator component. Completing its authority-aware new-app and existing-app workflows remains target work tracked by Issue #20. The target generator has two modes:

### New app
Create App Definition + Recipe.

### Existing app
Update an existing App Definition when needed + create a new Recipe.

Existing-app generation:
- package ID cannot change;
- existing artifacts change only through explicit author edits/replacement;
- the complete updated App Definition is revalidated;
- stale external edits are detected before publish;
- App Definition update and Recipe creation are one logical atomic operation;
- Recipe destination remains create-new/no-clobber.

Generator source mapping:

| Generator source/evidence | App Artifact source |
| --- | --- |
| local APK | `user_provided` |
| direct HTTPS APK | `direct_url` |
| repository/latest-release flow | `latest_release` |
| explicit selected release asset | `direct_url` to selected asset |

Selected permission automation is partitioned:
- non-root actions → `baseline`;
- root-required actions → `elevated`.

Generator inspection evidence is review-time data only and is not persisted as `metadata.apk_inspection`.

## 19. Catalog references and refactors

In the target architecture, App Definition IDs, App Artifact IDs, and App Target IDs are stable cross-document identities that may be renamed through explicit **catalog refactor** operations. Implementing these operations remains migration work tracked by Issue #22.

An App Definition ID rename:
- renames the App Definition ID;
- renames its canonical authored file;
- rewrites all dependent authored app references atomically.

Artifact/target renames rewrite all dependent references atomically.

Refactors:
- are explicit operations, not ordinary field edits;
- scan the exact dependent reference closure;
- refuse to proceed when any affected open document has unsaved changes;
- stage all resulting documents;
- validate the resulting affected set and relevant global invariants;
- publish all-or-nothing with rollback/recovery;
- do not rewrite unrelated canonical files.

Deletion differs from rename:
- referenced App Definitions cannot be deleted;
- referenced App Artifacts cannot be deleted;
- referenced App Targets cannot be deleted;
- deletion reports dependent authored locations and requires references to be removed/replaced explicitly first.

## 20. Validation scope

Ordinary App Definition saves validate:
- the edited App Definition;
- directly affected dependent Recipes;
- catalog-global invariants involving the edited identity/package.

Unrelated pre-existing catalog diagnostics do not block the save.

The implementation must remain simple and deterministic: there is no before/after "catalog health delta", no attempt to score whether an invalid catalog became better or worse, and no complex compensation semantics for unrelated existing errors.

Source overlap is generally advisory.

Blocking catalog conflicts include:
- duplicate App Definition ID;
- duplicate package ID;
- duplicate Recipe ID/destination;
- stale update/refactor conflict.

Same source URL, repository, latest-release policy, checksum, or other source overlap is warning-level rather than identity.

## 21. Existing authored-data migration

The final authored-data cutover updates all remaining app-aware authored data after the incremental migration slices are complete. Issue #23 tracks this unfinished cutover. The consequences below describe the accepted final v1 target.

Expected consequences include:
- create an ARMSX1 App Definition;
- migrate Obtainium, RetroArch, XaniteOG, and their Recipes;
- correct RetroArch package authority to `com.retroarch.aarch64`;
- move app-owned APK/source policy into App Definitions;
- migrate app-specific paths out of Recipes into App Targets;
- `feature.copy_bios` becomes RetroArch-aware because its current destination is intrinsically a RetroArch target;
- `feature.copy_roms` remains generic because its destination is user-configured;
- split generated permission actions into baseline/elevated App Definition sets;
- remove package/activity/permission duplication from app-aware Recipe steps.

At final cutover, architecture and generator documentation that describes App Definitions as non-authoritative metadata is replaced in place. The completed repository documents one authority model, not a legacy/current dual contract.
