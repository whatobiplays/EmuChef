# EmuChef Context

## Authority and implementation status

**Accepted normative product intent:** [EmuChef requirements](docs/product/emuchef-requirements.md) contains Q1–Q83 and UX-1 A, including generated configuration, device-profile OEM root-script execution, **atomic invocation admission/closure**, per-file recovery originals and cleanup. These are **accepted requirements, not yet implemented or physically qualified**; their presence in this document does not attest that the running app supports them.

**Architectural decisions:** [ADRs 0005–0011](docs/adr/) record accepted design boundaries and rationale. **Current implemented runtime behavior:** the detailed sections below and the [architecture documents](docs/architecture/) describe existing Rust/Tauri/sidecar code, which is authoritative for what currently works; do not infer code behavior from future product requirements. **Qualification evidence:** the [qualification-obligation register](docs/product/qualification-obligations.md) indexes open target/phase evidence, while original records, runbooks and validators decide actual passing, missing or deferred status. **Active scheduling/tracking:** [EmuChef wayfinder map #43](https://github.com/whatobiplays/EmuChef/issues/43) and its same 23 child issues own work order and direct body-level blockers, not execution authority or physical evidence.

**Scoped release authority:** The initial EmuChef proper public-release target is Apple Silicon macOS with applicable *current-product* Developer ID, notarization, Gatekeeper, clean-Mac, host artifact inspection, privacy/accessibility and selected supported-workflow evidence. See [release readiness](docs/release/release-readiness.md). The Config Editor public release and additional host platforms/cross-platform release automation are separately scoped/deferred, not unconditional prerequisites. Historical Config Editor signed artifacts remain evidence for those artifacts alone. This local documentation and GitHub tracking reconciliation does **not** authorize runtime work, commits/pushes, physical device operations, production enablement or release.

## Language

**EmuChef proper**:
The consumer application for choosing, reviewing, and applying a supported handheld setup. Simulation is optional rather than a prerequisite for an otherwise eligible reviewed apply.
_Avoid_: Config Editor, technical authoring UI

**App Configuration Definition**:
An app-owned description of supported configuration settings and their file format, independent of any particular setup procedure.
_Avoid_: Recipe-owned configuration format

**App Target**:
An app-owned named file or directory destination used when configuring that application; it is distinct from a user-selected storage destination.
_Avoid_: Storage Target

**Device Storage Volume**:
A storage resource available on the connected handheld, such as internal shared storage or a removable SD card.
_Avoid_: Hard-coded mount path

**Storage Target**:
A portable destination expressed as a selected Device Storage Volume and a relative location on it. A connected device supplies the concrete path when a setup is prepared.
_Avoid_: Literal SD-card UUID path

**Setup Location**:
A named, user-selected Storage Target shared by related Recipes and generated app configurations for the same setup.
_Avoid_: Recipe-local destination path

**Generated App Configuration**:
A rendered application configuration file produced from an app-owned definition or template, chosen settings, and resolved setup locations.
_Avoid_: Unreviewed device-side file generation

**Configuration Template**:
A reusable, app-scoped source configuration file containing declared variables or replaceable properties from which a Generated App Configuration can be rendered.
_Avoid_: Executable Recipe script

**App Recipe Configuration**:
An app-specific Recipe's configuration section, which references a base configuration file and selects the app-defined settings users may change.
_Avoid_: App Definition setting schema

**OEM Root Script Runner**:
A device manufacturer's facility for running a user-selected script with privileged authority, distinct from EmuChef obtaining root over ADB.
_Avoid_: Rooted ADB shell

**Privileged Setup Bundle**:
A run-bound set of staged files, generated root script and declared privileged actions prepared for execution using an OEM Root Script Runner.
_Avoid_: Rooted execution plan

**User-Mediated Privileged Execution**:
An EmuChef-reviewed execution stage performed when the user invokes an OEM Root Script Runner, rather than EmuChef directly running root ADB commands.
_Avoid_: Direct ADB root execution

**Privileged Execution Receipt**:
A run-bound report produced by the privileged script, distinct from independent postcondition verification.
_Avoid_: Verified device state

**Privileged Execution Stage**:
A reviewed contiguous group of privileged Recipe actions delegated to an OEM Root Script Runner; a setup may alternate multiple such stages with ordinary ADB work.
_Avoid_: Separate Recipe system

**Privileged Results Archive**:
A run-bound ZIP archive of privileged action outcomes and bounded diagnostics, stored separately from the staging bundle for retrieval and analysis.
_Avoid_: Staging bundle

**EmuChef Temporary Device Artifact**:
A device-side file or directory created and owned by EmuChef for staging, execution or result collection, which must be safely cleaned up when no longer needed.
_Avoid_: User content

**OEM Stage Barrier**:
A stage in a setup that pauses all other Recipe execution until user-mediated privileged execution and its result reconciliation are finished.
_Avoid_: Concurrent ADB Recipe execution

**Privileged Execution Attempt**:
One individual user invocation of a reviewed OEM root-script stage, with its own immutable results archive. Multiple attempts may belong to one stage.
_Avoid_: New Recipe or new setup

**Device Cleanup Obligation**:
An outstanding duty to remove EmuChef-owned temporary device files when safe and reachable, tracked independently from whether the setup itself succeeded.
_Avoid_: Failed setup

**Reviewed OEM Bundle**:
A frozen, target-bound script and payload set prepared for one reviewed privileged stage, including its effective backup mode. All invocations of the same bundle inherit those reviewed terms.
_Avoid_: Privileged Execution Attempt

**Execution Admission**:
The actual start of a privileged invocation as established by run-bound start evidence, distinct from staging or a user's acknowledgement of running it.
_Avoid_: Awaiting device action

**OEM Admission Window**:
The device-enforced authority state for a reviewed OEM bundle that admits safe serial attempts only while the stage barrier is held and admission is open.
_Avoid_: Host-side UI flag or staging file

**OEM Admission Closure**:
Atomic, device-proven revocation of new invocation starts for a reviewed OEM bundle; a racing start is admitted and reconciled or refused before mutation, and a retained closed script cannot run privileged actions.
_Avoid_: Deleting the script or releasing the host UI barrier

**Admission Closure Unverified**:
An unresolved execution-authority state where closure against new invocations or the absence of active/uncertain admitted attempts cannot be proven.
_Avoid_: Cleanup pending warning

**Recovery Original**:
The protected original bytes and restoration metadata retained in a qualified recovery store before an existing configuration is overwritten, distinct from temporary staging or result archives.
_Avoid_: ZIP result or staging backup

**Base Configuration Reference**:
A Recipe-owned pointer to reusable configuration content used as the starting point when generating a new application configuration.
_Avoid_: Duplicated app setting rules

**Saved Configuration Artifact**:
A retained, reusable copy of a rendered configuration, distinct from the immutable configuration snapshot approved for a particular execution.
_Avoid_: Current execution snapshot

**Configuration Library**:
A user-controlled collection of reusable configuration templates and explicitly saved rendered configuration files; it does not authorize automatic deployment.
_Avoid_: Reviewed Execution Plan

**Template Variable**:
A declared, typed value that a Configuration Template uses during rendering.
_Avoid_: Unvalidated textual substitution

**Scoped Recommendation**:
A suggested setup or setting value whose applicability is limited to a known device model or brand family.
_Avoid_: Universal default for device-specific behavior

**Explicit Setting Choice**:
A valid setting value intentionally selected by the user instead of accepting a device-specific or generic recommended value.
_Avoid_: Recommended default

**Compatibility Requirement**:
A setting constraint necessary for a supported device or application operation, distinct from an editable recommendation.
_Avoid_: Recommended default

**Managed Configuration Update**:
A change to declared application settings that preserves settings outside EmuChef's selected scope.
_Avoid_: Unconditional configuration replacement

**Configuration Recovery Copy**:
A protected copy of an existing application configuration file captured before EmuChef changes that file, available for explicit per-file restoration.
_Avoid_: Whole-setup rollback

**Add or Update Copy**:
A file transfer that creates or updates selected files without deleting unrelated destination content.
_Avoid_: Sync

**Replace Destination Copy**:
A file transfer that explicitly replaces the designated destination and its existing contents.
_Avoid_: Sync

**Mirror Copy**:
A file transfer whose destination contents match the source, including explicit removal of destination-only files.
_Avoid_: Sync


This document is the standalone current-state contract for the repository.

## Product

EmuChef provisions Android emulation handhelds from authored YAML. A device
plan selects recipes, the planner emits an execution plan, and the executor
applies that plan through local filesystem operations and ADB.

A **qualification operator** is an internal technical role that maintains the
evidence used to make device-support decisions. The role is not a primary
product user and does not define a third product workflow. **Qualification** is
internal engineering and evidence terminology; normal end-user compatibility
language is **supported** or **not supported**. Registration or evidence state
alone does not establish a user-facing support claim.

## Qualification Domain

**Device observation**: The trusted interpretation of the currently selected
device used by ordinary EmuChef behavior. A committed typed observation may
include inventory continuity, probe facts, authored profile matching, and
passive support/capability interpretation. Explicit root checking remains a
separate authority.

**Qualification session**: One internal evidence-capture attempt bound to a registered target, canonical qualification workflow, ordinary product intent, and authoritative product observations. At most one qualification session is active, and it may observe but never authorize the ordinary product workflow.

**Qualification candidate**: An immutable stored outcome of a closed qualification attempt. A candidate may be valid/passed, valid/failed, or invalid/not-observed; canonical evidence recording remains a separate explicit operator action.

Rust is the sole product runtime. The Cargo package
`crates/emuchef-rust-backend` builds the `emuchef` binary and owns:

- `plan`, `validate`, and `apply` CLI commands;
- authored catalog loading and validation;
- device profile matching and live `adb shell getprop` probing;
- execution-plan generation and single-threaded execution;
- dry-run and real-ADB adapters;
- the one-shot and JSONL editor protocols;
- the additive Phase 0 end-user catalog and execution-session protocol;
- read-only end-user device inventory, probing, deterministic matching, and
  product configuration operations;
- document sessions, command application, undo/redo, save, YAML emission, and
  reference indexing.

The `validate` CLI accepts either a recipe path or `--authored-root`. If both
are absent, it prints usage and an error to stderr, exits with status 2, and
writes no stdout. Recipe validation prints a text summary beginning with
`Validation status:`, followed by `Validated paths:` and any `Issues:`; the
`Errors:` count follows those sections.

The retired Python implementation, tests, dependencies, package metadata, and
entrypoints are absent from the repository. Python is not required for product
builds, development, tests, packaging, release qualification, or either Tauri
application. Repository policy rejects reintroduced Python product/runtime
code and alternate-backend selection paths.
The canonical retirement contract and evidence checklist are in
`docs/product/phase-4a-python-runtime-retirement.md`.

## GPT Repo MCP Product Contract

The repository-owned machine-readable product contract is
docs/product-contract.json. It is a bounded projection of PRODUCT.md and
the canonical product and qualification documents used for product-grounded
planning. Product decisions remain authoritative in the human-readable
documents; update the contract when those decisions change.

## Config Editor Authored Generation

The Config Editor provides guided generation workflows for a starter app
definition and installation recipe from a local APK, a public GitHub, GitLab, or
Codeberg/Forgejo repository or release, or a direct public HTTPS APK URL, and for a
starter device profile from one connected ADB device. Rust owns APK inspection
contracts, typed authored models, validation, canonical draft generation,
catalog collision detection and revalidation, evidence classification, and the
App-generator sidecar operations for combined admission, publication, and
rollback. Tauri owns native paths, configured external tools, exact ADB serials,
native save destinations, and trusted handles. React owns presentation and
explicit author choices.

Generation produces reviewable drafts and performs no authored-data writes until
explicit save. A save reports a confirmed publication, a confirmed failure, or an
unknown outcome when the sidecar response is lost after submission. A confirmed
save opens the new Recipe through the existing Recipe document session; a save
whose catalog writes committed but whose Recipe document could not be opened stays
a confirmed save with no document handle and reports the saved paths plus the
structured open error for manual recovery. An unknown outcome never resubmits
automatically and blocks saving until the author checks the recorded status or
explicitly builds a new review, which reuses the exact reviewed identities so the
backend still prevents a duplicate publication. App and device profile generation
remain draft workflows; App Definition
authoring also has a separate persistent document session for opening and
maintaining existing or standalone schema-v1 documents.
Schema-v1 App Definitions own intrinsic app identity and reusable app policy:
one package identity, app-owned artifacts and their source strategies, optional
baseline and elevated permission sets, App Targets, launcher activity, and
optional presentation metadata. A Recipe may bind a named App Definition
artifact with an `app_artifact` reference and attach app context to individual
steps with `app_ref`. Planning copies the referenced app's identity and
presentation into the execution plan and records late-bound release policy
there, so review and execution use the reviewed plan without reopening the app
catalog. App-backed artifact references, APK installation, and package-state
conditions require the reviewed App Definition context. Package-state
conditions read the package identity from their bound App Definition and use
an empty parameter map; legacy package-name conditions and app-less APK
installation forms are invalid.

The RetroArch App Definition owns named `kind: file` direct-URL artifacts for
seven frontend archives, thirteen core archives, and three core-system
archives. Its provisioning Recipe references these resources through the
existing Recipe artifact IDs and keeps cache selection, artifact groups,
extraction, step dependencies, and app-aware copy destinations. The Recipe's
`retroarch_apk` is a Recipe App Artifact reference to RetroArch's `apk`
App Artifact. The install step carries `app_ref: retroarch`, and its
`package_installed` condition is parameterless so package identity comes from
the App Definition.

An App Definition target names a file or directory at `app_data`,
`external_app_data`, `shared_storage`, or `absolute_device_path`. App-aware
`copy_files.dest` and compatible `path_exists` or `file_exists` conditions use
the single-key `{ app_target: <target-id> }` value. The enclosing step's
`app_ref` supplies the owning App Definition. Planning resolves user-0 paths,
validates source and destination kinds and app provenance, and derives
`app_data_write` or `shared_storage_write` before capability selection. The
execution plan retains concrete paths and effective capability constraints;
review and execution do not resolve authored target IDs. Generic copy
destinations and the user-configured ROM-copy Recipe retain their existing
literal or input-reference behavior.

App-aware `grant_permissions` steps name an App Definition with `app_ref` and
select exactly one `permission_set`: `baseline` or `elevated`. Their
`policy.on_failure` is `warn` or `fail`. The planner resolves the selected
App Definition actions and target API applicability into the step-local
`ExecutionStep.permission_plan`; that plan is the only executable action
source, contains the reviewed package identity and per-action applicability,
and is covered by the canonical execution-plan digest. Canonical schema-v1
`params` do not contain duplicate runtime or app-op action arrays. Review, execution, and
reporting read the same retained plan and app snapshot.

Bounded actions cannot be planned when the Android API level is unknown;
planning reports the action and bounds that need resolution. An action without
API bounds remains applicable with an unknown API level. A confirmed API level
outside an action's inclusive bounds retains that action with a
`not_applicable` reason. A selected permission step whose actions are all
inapplicable is reported as skipped, with each reason retained and no shell
execution or root preflight. A capability-pruned step is absent from the plan.
When the selected target supplies an Android API level, that target fact takes
precedence over profile context and is retained in the plan's device context
for review.

`baseline` requires `shell_command` and executes through ordinary ADB shell.
`elevated` requires both `shell_command` and `root_shell` and is omitted when
either capability is unavailable. Reviewed root authorization is required only
when an elevated plan has at least one applicable action; an all-inapplicable
plan skips without root qualification or execution preflight. The real adapter
separately checks authorization and revalidates root immediately before each
privileged command. The fixed command surface grants runtime permissions or
sets app-ops to `allow`; it does not accept authored shell commands.
Ordinary action failures continue across independent applicable actions. Under
`warn`, each failed action is reported as a warning and the step succeeds with
warnings; under `fail`, the step fails with an execution error. Root-authority,
identity, transport, and interruption failures stop execution immediately.
ADB command, identity, and transport failures use generic public result
messages without command arguments or stderr. Bounded, redacted command
arguments and stderr remain available as internal diagnostics, and stdout is
not retained in those diagnostics. Root revocation is reported as
`Root authority was revoked during execution.`
Permission automation is App Definition-owned. A Recipe `grant_permissions`
step names `app_ref`, selects a declared `baseline` or `elevated`
`permission_set`, and may set its failure policy. Generators create these steps
only when the author explicitly enables applicable permission automation.

RetroArch places `MANAGE_EXTERNAL_STORAGE` in its App Definition's `elevated`
set, so all-files access automation runs only on rooted devices. Unrooted
devices still run its applicable baseline runtime permission actions and
continue through launch because the launch depends on the baseline step only.
When both permission steps are selected, the stable topological sort preserves
their authored baseline-then-elevated order before launch.
Configuration discovery derives each Recipe's availability through the Rust
planner with the same device context, bindings, App Definitions, target
resolution, and step dependency rules used for execution planning. The planner
is the only authority for effective App Target and permission capabilities,
Android API applicability, runtime capability masks, dependency pruning, and
optional-input pruning. A Recipe with retained planner steps is available. A
genuinely empty Recipe and a Recipe whose optional inputs omit all work retain
their available status. If capability pruning removes all otherwise-valid
executable work, the Recipe is unavailable and `unavailableCapabilities`
contains only missing runtime capabilities that can restore valid candidate
work. This flat list describes possible blockers: independent work paths can
become runnable through different capabilities, so the list does not mean all
entries are cumulative requirements. A Recipe with an unresolved App Target,
invalid permission authority, invalid dependency graph, or ambiguous App
Definitions is unavailable with a diagnostic identifying the affected
Recipe and step; discovery never substitutes authored-only constraints when
planner resolution fails. Target-resolution diagnostics block the affected
selected or dependency-required Recipe, while unrelated unselected Recipe
diagnostics remain warnings. An API-inapplicable permission action remains in
the plan as a skipped no-op and contributes neither `shell_command` nor
`root_shell`; applicable permission actions retain their existing capability
gates.

A Recipe step that names an unsupported runtime capability is pruned with a
Recipe-keyed diagnostic. The Recipe remains available when other planner-retained
work can run. If unsupported requirements leave no retained work, the Recipe is
unavailable; unsupported names are not reported as missing device capabilities.
A Recipe that selects a valid user-provided App Artifact remains available while
its required file input is unbound, while `planConfiguration` continues to reject
that missing binding. Malformed App Artifact references, invalid input
declarations, and invalid App Definition authority fail closed.

App-aware `launch_app` and `force_stop_app` steps require `app_ref` and no
authored package or activity parameters. An App Definition launcher activity is
one fully qualified Android class name. Planning requires it for app-aware
launch, then seals the package and activity as literal values beside the App
snapshot in the reviewed plan. Force-stop seals the package alone. Execution
and retained final-launch eligibility verify the plan-local snapshot and
concrete target before contacting the device. Schema-v1 lifecycle steps
resolve package identity from the referenced App Definition; launch also
resolves its launcher activity there. Package and activity literals are not
accepted as lifecycle authority. Config Editor exposes an
editable App Definition ID for launch and force-stop steps, while other step
types retain their existing read-only display of an assigned context. Changing
the ID preserves any authored parameters so the author can clear them
explicitly. An empty field removes `app_ref`, and Rust validation supplies
diagnostics for incomplete forms.

App Definition `direct_url` sources for `apk` and `file` artifacts require a
public HTTPS URL without credentials or a fragment; signed query parameters
are allowed. A direct URL does not pin a release or guarantee immutable bytes.
The App Definition may include an expected SHA-256, which the artifact
resolver checks against the returned
artifact after validating source policy, including on cache hits. The resolver
calculates the SHA-256 for every direct-URL resolution and gives execution a
verified per-run snapshot so later cache changes cannot alter bytes passed to
APK inspection or installation, or file extraction and copy steps. APK
package-identity verification applies only to `apk` artifacts; `file` artifacts
are not package-verified. Durable execution reports include Recipe and App
Definition artifact identities, filename, calculated SHA-256, cache status, and
a redacted observed final URL only for network transfers. A calculated
checksum without a trusted expected value is not publisher authentication.
Hostname source addresses are resolved for each direct HTTPS request and
redirect. Every resolved address must pass the public-address policy, and the
accepted addresses are pinned for direct connections while normal TLS
certificate and hostname verification remains enabled. A configured system
proxy remains enabled; proxy-side DNS is controlled by that trusted network
route and cannot be pinned by the local resolver. Execution records retain the
exact approved plan, including its original URL query, with the matching plan
digest. Execution records retain the exact reviewed plan, including sensitive
input values. Consumer-facing reports redact sensitive values in both
executable inputs and review snapshots while preserving the original plan and
digest internally. When any consumer redaction is needed, the report omits
`reviewedPlan` and serializes `redactedReviewedPlan`. The
`reviewedPlanSourceUrlsRedacted` marker is present only when a source URL was
redacted; input-only redaction does not set it.

App-owned `latest_release` artifacts support GitHub, GitLab, and Forgejo from
their authored public HTTPS service origin and repository. Review records the
provider policy without resolving a release; execution resolves the latest
eligible release before selecting an asset or consulting the artifact cache.
Draft GitHub and Forgejo releases and upcoming GitLab releases are excluded.
GitHub eligibility follows the provider's prerelease flag and ranks by parsed
`published_at`. GitLab has no first-class prerelease flag, so its approved
best-effort classification checks the lowercase concatenated tag and release
name for `alpha`, `beta`, `preview`, `prerelease`, or `pre-release`, or the
`rc` token; it ranks by parsed `released_at`. Forgejo uses parsed
`published_at`, falling back to parsed `created_at` when needed. Missing or
malformed timestamps do not rank as valid GitHub candidates; non-upcoming
GitLab releases with missing or malformed `released_at` fail closed; Forgejo
requires at least one usable publication or creation timestamp. Equal
timestamps use deterministic tag and provider-identity tie-breaking.

Release discovery is bounded to five pages of thirty releases. GitLab requests
descending `released_at` order and can stop once a selected eligible release is
strictly newer than every release on the current page. GitHub and Forgejo
continue according to their pagination markers because their list endpoints
do not provide a relied-upon timestamp-order guarantee. Pagination endpoints
and links must remain on the selected provider API origin and expected release
path. Exhausted results with no eligible release, ambiguous pagination, or a
history that cannot establish the latest eligible release within the bound
fail with stable errors. The resolver selects the latest eligible release
first, then filters its assets by the App Artifact kind (`apk` or `file`) and
optional filename-only regular expression; inversion defaults to false and is
invalid without a pattern. Exactly one asset must remain. The version-1
Execution Plan preserves the optional pattern and default inversion semantics.

The selected asset URL is part of the existing artifact-scoped cache identity,
so a cached older release cannot replace release discovery. Release metadata
and selected downloads use the public HTTPS policy, including public-address
checks, TLS verification, redirect validation, bounded responses, sandbox
admission, and atomic publication. Runtime admission verifies App Artifact
provenance and kind against the plan snapshot. Review describes the policy for
the exact artifact referenced by the installation step. Execution reports
include the selected release and asset, calculated SHA-256, cache status, and a
redacted final URL only when a network download observed one; cache hits do not
claim a network URL.

Android package facts come from APK inspection rather than filenames. The
APK-inspection contract uses a separately configured user-supplied
`apkanalyzer` or `aapt2`; EmuChef does not bundle Android SDK build tools.
Local APK generation uses a required user-provided APK recipe input, and its
generated App Definition declares one named `apk` artifact whose source strategy
is `user_provided`. App Artifact IDs and Recipe IDs are explicit author edits: the
generator proposes an artifact ID (`apk`) and the Recipe ID
`app.<app-id>.<artifact-id>.install`, and it never silently renames an identity the
author replaced. A colliding App Artifact ID or Recipe ID is a blocking review
diagnostic. The selected APK path is session-only. The last validated
analyzer executable and authored root are persisted by the trusted Tauri layer
so later generator sessions can restore them. Verified APK inspection facts are
transient draft review evidence and are never written into the saved App
Definition.

The local APK wizard accepts regular `.apk` files no larger than 2 GiB and a
regular executable whose basename matches the selected analyzer adapter.
Analyzer commands use direct arguments, a 30-second timeout, and a 4 MiB bound
per output stream. The current analyzer command surfaces do not provide a
signing-certificate SHA-256 fingerprint, so that fact remains missing with a
deterministic warning. Split and non-base APKs are rejected.

Remote source analysis runs only in the trusted Tauri layer. GitHub modes use
the GitHub REST API and never scrape rendered HTML. Draft releases are always
excluded; repository mode excludes prereleases by default, while exact
prerelease selections require explicit confirmation. Candidate assets must be
non-empty `.apk` files no larger than 2 GiB. Direct URL mode requires a public
HTTPS address without credentials or a fragment and preserves the original query.
Provider and repository addresses reject query parameters, which are refused
rather than silently dropped. Metadata responses are limited to 2 MiB, redirects
to five HTTPS hops, connection time
to 10 seconds, and complete requests to 30 seconds. APK downloads stream into
a generator-session temporary directory and are removed when that session is
cancelled, completed, restarted, or dropped.

Remote sources generate an app-aware Recipe that references the selected App
Artifact through a Recipe `app_artifact` reference. The generated App
Definition records that artifact's durable source: `direct_url` with the
normalized public HTTPS APK URL, covering both a direct APK URL and a pinned
release asset, plus the lowercase trusted publisher SHA-256 when the author
supplied one; `latest_release` with provider, service origin, repository,
optional asset filename pattern, and prerelease policy for latest-compatible
mode; or `user_provided` for the same user-provided APK recipe shape used by
local generation. The Recipe's install, permission, and launch steps use the
App Definition's app reference, and user-provided sources also
declare the local APK file input that binds to it. Credentials, temporary paths,
response bodies, and inspected APK facts are
not persisted. The reviewed App Definition artifact owns the download policy;
the generated Recipe references that artifact without copying the source
policy, so the filename pattern and prerelease toggle in the source step mirror
that row while a draft exists and
the release preview, review gate, and save gate follow it. Only a
latest-release source carries that policy, so a pinned artifact never resets
either control. Generation blocks when that artifact is removed or replaced. Only
an `apk` artifact can represent the reviewed source, so retyping it as a generic
`file` blocks generation, and source-step edits rewrite only the single APK
artifact whose ID equals the reviewed App Artifact ID carried by the request, so
an unresolvable pairing leaves every authored row untouched. A prerelease toggle
that keeps the selected asset eligible keeps the rows it is derived from.
Starting a new
remote download clears the source-step publisher checksum, because the freshly
downloaded artifact has no author-supplied publisher checksum, and only the
paired artifact receives source-step edits. Release
analysis retained from an earlier source is presented only while it still
describes the repository the paired artifact resolves from; a retargeted
source stops using that
preview, the review and save gates do not treat it as authoritative, and
Rust reports a non-blocking `latest_release_analysis_stale` warning. Local
generation blocks the other way around: structured mapping edits must leave
exactly one user-provided APK artifact for the recipe's user-provided input.
Because the runtime release resolver resolves GitHub and GitLab releases from
their official service origin, generated drafts accept only
`https://github.com` for `github` and `https://gitlab.com` for `gitlab`;
`forgejo` drafts resolve from the authored origin.

App-generator sessions retain local and downloaded APKs, remote source and
asset selections, analyzer paths, temporary workspaces, and authored-root
paths behind opaque process-memory handles. The Config Editor also maintains one validated,
persistent app-wide authored-root selection. Selecting a root in app generation
updates that shared selection, applies it to an open recipe document, and makes
it available to later app-generator and device-profile-generator sessions.
Authored roots must contain existing canonical `apps`, `recipes`, and applicable
generator destination directories. Generation authors either a new App or adds one
APK artifact to an existing App selected through an opaque trusted handle.
Existing-App authoring preserves the App's identity, facts, definition, and other artifacts,
locks its App ID and package, and
reports an actionable blocking error when the inspected APK package does not match
the selected App; the author resolves that by creating a different App rather than
updating this one. Publication rechecks APK file identity, revalidates both typed drafts, reruns
collision analysis, commits the Recipe first with no-clobber semantics and the App
file last without cross-file atomic visibility, and removes a first commit only while the
operation still owns the commit and the file's filesystem identity and content still
match the file this operation created, otherwise preserving the Recipe
and reporting it for recovery. Publication refuses to publish while another active
session holds unsaved edits to the target App document, which leaves the generator
draft intact. A confirmed save opens the generated Recipe
through the existing Recipe document session when the backend returns its document
ID and invalidates the generator session. The Config Editor permits only one app or
device generator wizard at a time.

Generated recipes are minimal: an APK input binding and artifact resolution when
needed, an APK installation step that references the App Definition, a
parameterless verified package-installed skip condition, App-owned permission
automation only when the author opts in, and an optional launch-once step whose
opt-in is independent of a stored verified launcher activity. Generation does not
infer configuration-copy, root, permission, force-stop, app-data, or device-plan
behavior from repository prose.

Permission review partitions inspected candidates into baseline (no root) and
elevated (root-required, optional) sets, sends only the selected identities with
the opaque inspection handle, and adds actions to App-owned policy only after an
explicit automation opt-in. The review reports the whole resulting baseline and
elevated sets, omits empty sets, and reports each addition's effect on existing
Recipes. Inspected permission and source facts remain transient review evidence.
Editing a reviewed identity, root, target App, or source invalidates the reviewed
output, which must be rebuilt before saving.

Device profile generation is available from the Config Editor File menu as an
ephemeral guided wizard. Tauri invokes the existing device-listing and probing
operations with literal `adb` resolved from `PATH`. Generator-session handles
scope every selected device and the generator-local root handle; exact ADB
serials remain in trusted process memory and never enter React state, generated
YAML, or product-facing errors. The generator can bind the existing app-wide
authored-root selection into a fresh opaque session handle, and selecting a new
root updates that shared editor setting. Generator UI receives only safe device
facts and a constant authored-root label.

Standard capture is read-only and includes manufacturer, brand, model, product,
device, board, hardware, ordered de-duplicated ABI values, Android major
version, and Android API level. Drafts propose exact manufacturer and brand
tokens, a regex-escaped anchored model pattern, and Android major as the minimum
without a maximum. Schema version 1 and `device_profile` kind are fixed; all
other proposed authored values remain editable with their original evidence
classification and an edited-from-proposal marker.

Device-profile draft generation, canonical YAML emission, validation, and
collision analysis are side-effect free. Existing ID and destination conflicts
block saving; likely matching overlap warns. An incomplete collision scan still
allows draft review but produces a blocking diagnostic. Final save revalidates
and rescans, derives the destination under the selected root's existing
`device_profiles` directory, rejects overwrite and path escapes, and publishes a
synced temporary sibling with atomic create-new/no-clobber semantics. Extended
shared-storage, package-manager, activity-manager, root, APK, or other capability
checks are not implemented.

Typed schema-v1 `AppDefinitionV1` and `DeviceProfileV1` models are the shared
authority for generator output, structural parsing, canonical emission, save
validation, catalog loading, and future dedicated editors. App Definition v1
requires `id`, `name`, and `package_id`; allows optional `description`,
`category`, `launcher_activity`, and `metadata`; and models artifacts as named
`apk` or `file` entries whose sources use exactly the `user_provided`,
`direct_url`, or `latest_release` strategies. Retired fields such as package
aliases, install or tracking sources, provisioning flags, and persisted
`metadata.apk_inspection` are rejected instead of migrated by a compatibility
reader. The `artifacts` and `targets` registries reject a repeated entry id
while parsing, because YAML would otherwise keep only the last definition. An
artifact source that supplies a key belonging to a different strategy is
rejected, and an explicit `null` for such a key counts as supplied rather
than absent, while `null` for an optional key of the selected strategy is
equivalent to omitting it.
Catalog loading validates every App Definition and enforces a globally
unique App Definition ID and `package_id`; the product `describeCatalog` result
includes an `apps` projection carrying only id, name, description, category,
and package id, and an App Definition catalog that violates per-file or
catalog-global App Definition invariants fails product catalog loading.
Proposed values retain verified, derived, suggested, or missing evidence in
draft DTOs; author edits do not replace that provenance, and final YAML
contains only reviewed authored values. The complete approved plan is
documented in `docs/product/config-editor-authored-generation.md`.

## Authored Data and Planning

Authored source lives under `authored/`:

- `apps/` defines App Definitions that own intrinsic app identity and policy;
- `recipes/` defines inputs, artifacts, groups, and ordered steps;
- `device_profiles/` defines match criteria and capability defaults;
- `device_plans/` selects profiles and recipes.

### ARMSX1 latest-release source policy

The `armsx1` App Definition resolves only stable GitHub releases from
`ARMSX2/ARMSX1`, excludes draft and prerelease releases, and requires package
identity `com.nanodata.armsx`. Its anchored APK policy accepts the complete
filenames `ARMSX1-release-YYYYMMDD-arm64-v8a.apk` and
`ARMSX1-MAJOR.MINOR.PATCH.apk`. Debug, source, nightly, and malformed filename
forms remain ineligible.

Selection first identifies the newest eligible stable release and then applies
the App-owned asset policy to that release. A newer eligible release without a
matching APK fails with `remote_asset_no_match`; selection does not fall back
to an older release. Multiple matching APKs in the selected release fail as
ambiguous.

Product catalog operations consume a resolved local snapshot rather than a
repository-specific global root. A snapshot carries `bundled` or
`local_directory` source kind, source id, optional version and cache key, and an
independent optional SHA-256 content digest. `cached_remote` is reserved but
has no resolver, networking, synchronization, signature, or update behavior.
The legacy `authoredRoot` request field remains a compatibility adapter for
existing configuration operations; new product catalog operations require a
resolved `catalog` object.

`emuchef plan` accepts an authored root; either a device-plan id or a saved
user-configuration ID/path; an optional configuration-root override; optional
recipe and input-binding overrides; optional explicit device context; optional
ADB path/serial; an output path; and verbose output. Supplying `--adb` or
`--serial` enables live device probing. Generated execution plans use
`plan.<device-plan>.001` identifiers. No `--recipe` and no `--clear-recipes`
leaves request selection absent, one or more `--recipe` values replace saved or
device-plan selection, and `--clear-recipes` supplies an explicit empty
replacement. The clear flag conflicts with any recipe occurrence.

Each `--bind <recipe-id>/<input-id>=<JSON_OR_STRING>` key may occur once.
Values parse as JSON when valid and otherwise remain strings, so list and
`multiple: true` inputs use JSON arrays. Duplicate explicit keys are CLI errors
and are never converted into arrays.

Raw one-shot and sidecar requests for `describeConfiguration` and
`planConfiguration` reject duplicate qualified keys in a `bindings` object
before JSON object entries are collapsed into a map. The failure keeps the
existing `invalid_request` code and reports `duplicate_binding_key`, the
`bindings` field, and the qualified key in structured details without either
supplied value. CLI duplicate handling remains a separate argument-parsing
contract.

Step parameter specifications declare their accepted value sources and runtime
value types. Authored recipe refs remain recipe-local (`inputs.<id>`,
`artifacts.<id>.<field>`, and `steps.<id>[.outputs.<name>]`). Validation rejects
literal or ref namespaces that a parameter does not accept and rejects refs
whose declared runtime type is incompatible. `copy_files.dest` accepts a
literal device path, compatible input ref, or typed App Target;
`copy_files.copy_policy` accepts a literal or compatible input ref, while
`copy_files.source` remains limited to compatible input, artifact, and step
output refs. Canonical recipe YAML continues emitting direct literal values and
single-field `{ ref: ... }` and `{ app_target: ... }` mappings.

Recipe input declarations support `string`, `integer`, `boolean`, `enum`,
`file`, `directory`, `path`, `device_path`, `string_list`, `path_list`, and
`object`. Each declaration carries semantic label, description, and role
metadata; required and multiple flags; a JSON-compatible default; structured
enum options; sensitive and advanced presentation flags; extensible metadata;
and validation for existence, extensions, path kind, and allowed device-path
prefixes. Enum option values are unique, defaults match the declared type and
enum options, and canonical YAML preserves declaration and option order.

Planner runtime types map host files and directories to `file_path` and
`directory_path`, device paths to `device_path`, enums to strings, and declared
list types to typed arrays. Device paths retain device location and are not
expanded or rewritten as host paths. The authored `feature.copy_roms` recipe
demonstrates a host directory, constrained device destination, and enum policy
consumed through recipe-local input refs.

Schema-v1 user configurations persist recipe selection and direct runtime input
bindings independently from authored recipes. A document requires a non-empty
`device_plan`, keeps fully qualified `<recipe-id>/<input-id>` binding keys, and
preserves unknown top-level extension fields deterministically. Structural
loading is catalog-independent: malformed YAML, unsupported schema identity,
invalid required field shapes, malformed binding keys, duplicate mappings, and
ambiguous binding value sources prevent loading. Recipe and input existence,
dependency-expanded selection membership, value compatibility, enum and path
constraints, device-plan existence, and missing required inputs are semantic
diagnostics, so a structurally valid document remains editable and canonically
emittable when its catalog is unavailable or its saved values are invalid.

User-configuration identifiers resolve as `<configuration-root>/<id>.yaml`.
The configuration root is the directory containing configuration files and may
be set explicitly. Otherwise EmuChef uses the platform configuration directory:
`~/Library/Application Support/EmuChef/user-configurations` on macOS,
`%APPDATA%\EmuChef\user-configurations` on Windows, and
`$XDG_CONFIG_HOME/emuchef/user-configurations` or
`~/.config/emuchef/user-configurations` on Linux. Absolute values, values with a
slash or backslash, and case-insensitive `.yaml` or `.yml` suffixes are always
paths, including when missing. Other values must be valid configuration IDs;
resolution never searches the authored root or current working directory.

Planning keeps explicit request bindings, persisted user-configuration
bindings, and device-plan input overrides as separate maps until the planner
resolves them. For every input in the dependency-expanded selected recipe set,
the winning value is chosen in this order: explicit request, saved user
configuration, device plan, recipe default, then unbound. Only the winning
value is validated for a planning request; a valid higher-precedence value may
therefore shadow an invalid saved value. An invalid winner produces a
diagnostic and never falls back. Resolved records retain `explicit`,
`user_configuration`, `device_plan`, or `recipe_default` provenance. Unknown
keys and bindings outside the effective recipe set are errors, optional inputs
may remain explicitly unbound, and required inputs without a winner are
reported together. The execution plan receives normalized values selected for
execution in its `inputs` field; the executor does not merge binding layers or
interpret provenance. Product plans also retain immutable `resolved_inputs`
review snapshots for every input in dependency-expanded Recipes, including
unselected and nullable values, the winning binding source, and the input's
review presentation. Device Plan ID/name/description and expanded Recipe
ID/name/description are captured in the same plan. Review uses these snapshots
and planning-time diagnostics without reopening authored Device Plans or
Recipes. The canonical plan digest covers the snapshots, so a fresh plan
captures presentation edits and receives a new digest.

The side-effect-free `describeConfiguration` JSON operation accepts an authored
root plus a request or saved device plan, optional saved configuration,
syntax-resolved configuration root, optional selected-recipe replacement,
explicit bindings, and device context. Request-level `devicePlan` and
`selectedRecipes` fields replace saved values when present, including an
explicit empty recipe list. Discovery expands dependencies, applies the central
binding resolver, and returns every effective input with declaration metadata,
partial values, provenance, per-input diagnostics, and aggregate diagnostics.
It does not require a complete valid plan and performs no ADB, network,
extraction, copy, device-write, or persistence operations.

For both discovery and planning, the camelCase protocol field
`userConfiguration` accepts either an ID/path string or an inline schema-v1
document object. Inline documents use the persisted schema's snake_case fields,
including `schema_version`, `device_plan`, and `selected_recipes`; camelCase
aliases inside the document are not accepted. Inline objects use the same
structural parser and model as file-backed documents and perform no
configuration-file lookup.

The `planConfiguration` JSON operation accepts the same context and performs
complete catalog, dependency, winning-binding, ref, constraint, and required
value validation before returning an in-memory normalized execution plan,
resolved inputs, diagnostics, and a backend-authored user-facing review
projection. The projection uses authored setup, feature, action, and input
labels; groups populated preparation, download, install, copy, permission,
launch, and device-change sections in deterministic order; and includes only
authoritative action counts, known waits, device destinations, warnings, and
blockers. Sensitive inputs appear only as `Provided`; approved portable host
inputs appear as filenames or counts rather than host paths. It performs no ADB commands, downloads,
extraction, host or device copies, writes, or plan-file persistence. A
request-level `devicePlan` replaces the saved required `device_plan`; the saved
document itself remains unchanged and self-contained. The desktop editor can
request and inspect this structured plan result without shelling out through
the CLI.

Product planning returns canonical JSON SHA-256 `planDigest` data and captures
resolved catalog identity, a reviewed target binding, ordered recipe
name/description snapshots, and normalized step notes. Canonical plan JSON
recursively sorts object keys, emits no insignificant whitespace, and preserves
array order. Target serial compares exactly after trimming; manufacturer and
model compare case-insensitively after whitespace normalization; Android API
level compares exactly. Missing or different actual values are hard failures
when the reviewed plan contains the fact.

Authored steps may include optional `progress_note` presentation text. Runtime
fallback order is progress note, step name, humanized step type, then step id.
This metadata does not change planner selection, dependencies, parameters,
execution, or verification. Existing recipes remain valid without it.

## Execution

Execution is single-threaded and dependency-aware. A failed step blocks
dependent steps while unrelated steps may continue. Skip conditions produce
skipped steps according to the execution-plan contract. Verification runs after
the step action and can fail the step.

The JSONL sidecar's additive `phase0_end_user_runtime` extension is explicitly
negotiated. It supports `describeCatalog`, `startExecution`, `getExecution`,
`getExecutionEvents`, `cancelExecution`, `listAdbDevices`, `probeDevice`, and
`matchDevice` while retaining the existing configuration operations. One
execution may be active in a sidecar process;
terminal attempts remain inspectable in memory until process exit.

## End-User Support and Artifact Cache

The end-user Tauri application owns a dedicated artifact cache at
`<app-data>/artifact-cache`. Tauri derives and injects that root when it starts
the sidecar. React, environment variables, and current-working-directory policy
cannot redirect it. The backend default remains `.emuchef_cache/artifacts` when
no explicit cache root is supplied, so CLI, config-editor, tests, and other
embedders retain their existing behavior. Legacy caches are neither migrated
nor deleted.

Each managed cache entry consists of one payload and an optional schema-v1
metadata sidecar. The sidecar contains a safe artifact label, source-kind enum,
SHA-256 source fingerprint, expected payload size, and internal payload
filename plus the payload modification fingerprint captured before promotion;
it contains no raw URL or path and provides no execution authority.
Metadata publication failure leaves a usable unindexed payload and does not
fail execution. Inventory count and size treat payload and metadata as one
logical entry. Orphan metadata and unrecognized files remain unmanaged and
non-removable.

React receives generation-scoped opaque cache-entry handles plus safe category,
label, source kind, structural integrity state, combined size, age bucket,
in-use state, and removability. Tauri owns selective, unused, and all-removable
cleanup. It requires exact count/size confirmation, blocks cleanup while an
execution is starting or active, and revalidates root confinement, symlink
status, logical association, and both component fingerprints immediately before
deletion. Cleanup returns stable sanitized outcomes and a fresh inventory.

The Troubleshooting panel exports a deterministic schema-v1 ZIP through a
native save dialog. Tauri owns the destination and bundle bytes. The 2 MiB
bundle includes only app/runtime status, OS class, feature gates, public catalog
identity, aggregate saved-configuration and retained-execution state, and
aggregate cache counts/sizes. It excludes names, configuration contents,
bindings, handles, paths, serials, environment values, URLs, credentials, logs,
process or ADB output, reviewed plans, files, and crash data.

`startExecution` requires a complete plan and its canonical digest, recomputes
the digest before apply, and rejects `runtimeRoot` and `cacheRoot`. Those roots
are configured at sidecar startup and each attempt derives its own runtime
directory. Real execution requires reviewed target facts and performs ADB
preflight. Dry-run performs fake-device execution only and reports
`simulated: true` with `simulated_only` verification scope; it is not
real-device verification.

After digest validation and real-target preflight, `startExecution` performs
canonical admission for every retained artifact before committing an execution
number, report, event, active record, cancellation state, or worker. Admission
runs under the execution-state lock without creating persistent reservation
state or reacquiring execution state. It shares the resolver's artifact type,
cache mode, destination, URL, local-source, and sandbox policy; performs no
network or filesystem/device mutation; and leaves the execution number and
single-active slot unused on failure. Safe failures use
`execution_start_failed`, `artifact_not_ready`, and a stable artifact cause code
without artifact identifiers, URLs, credentials, paths, roots, or raw sandbox
details.

Execution reports contain the full reviewed plan, digest, target, RFC 3339 UTC
timestamps, overall status, structured issues, ordered recipe groups with
captured names/descriptions, and ordered step results with recipe ownership,
notes, messages, and outputs. Overall status is `running`, `succeeded`,
`succeeded_with_warnings`, `failed`, or `cancelled`. Blocked required work makes
the overall attempt failed; `blocked` is primarily a step or recipe-group
state. Incremental events use per-execution sequence numbers and
`getExecutionEvents(afterSequence)` while `getExecution` remains the complete
snapshot recovery surface.

Cancellation is cooperative between atomic operations. It stops new step
scheduling, may allow the current operation to finish, preserves completed
results, marks unscheduled work cancelled, and performs no rollback. A worker
panic yields `execution_worker_panicked`, leaves a terminal inspectable report,
and releases the active slot. Retry or repair means generating and reviewing a
fresh plan and starting a new execution id. There is no runtime rollback,
device-state undo, reverse-step generation, automatic backup, or restoration
promise.

Completed ADB/device commands with bounded, anchored no-space evidence use the
private `device_storage_exhausted` failure kind and stable issue code. The
classifier accepts recognized ENOSPC output from either bounded stdout or
stderr and either completed exit code, but never overrides timeout, process,
transport, identity, or root failures and never matches arbitrary filenames or
echoed prose. Storage exhaustion fails the active real step, preserves earlier
evidence and conservative possible-partial-change reporting, stops later
device/host work, leaves never-started work pending for terminal **Not
attempted** projection, releases the active slot, and requires fresh device
qualification, plan generation, review, and execution after the user frees
space. EmuChef never deletes user data, retries, resumes, or continues the old
execution automatically. Tauri exposes only authored storage guidance and no
raw ADB text, paths, serials, or command arguments.

Physical storage qualification has a separate dependency-free operator utility
at `tools/device-storage-preflight.mjs`. Its immutable
`phase-6d6-low-storage` profile requires exactly one selected authorized ADB
device and `com.emuchef.fixture`, verifies that `/sdcard/Download` and the
qualification destination share the same reported filesystem and mount, and
uses device-local `dd` to create non-sparse chunks only under the exact marked
`/sdcard/Download/EmuChefStoragePreflight/phase-6d6-low-storage` directory.
Status and dry-run are non-mutating; preparation requires explicit confirmation,
remeasures after each chunk, stops inside the 4,194,304–5,308,416 KiB window,
and resumes only an exactly marked directory containing recognized chunk
names. Cleanup rejects symlinks, missing or mismatched markers, and unknown
entries; it removes only the owned profile directory, synchronizes the device,
verifies absence, and reports restored capacity. This preflight allocation is
operator setup rather than physical evidence or production execution. It
remains across both low-storage repetitions while the harness independently
owns and cleans its run-scoped reserve, filler, payload, and sentinel state.

Host suspension does not create a second execution owner. The same locally
owned child/future tree remains authoritative if the process generation
survives sleep; after wake the existing operation may complete, report typed
transport loss, or reach `operation_timed_out` according to the fixed Rust
deadline semantics. If the sidecar generation is lost, the terminal outcome is
`runtime_session_lost` and Tauri projects `execution_unavailable` with an
indeterminate real-device outcome. Application restart never resumes an
execution. Host-sleep qualification measures whether the existing timer source
observes suspended time on the qualified host. Passing evidence requires
samples from the production deadline clock at start, before sleep, after wake,
and terminal; remaining budget before and after suspension; wall duration;
tolerance and rationale; and the scenario phase. Classification is derived
from clock advancement and budget consumption, never from the terminal result.
Excluded suspension may still time out after later active time. Transport loss,
missing samples, or inconsistent measurements block qualification. The owned
process timer now shares one exact monotonic start/deadline basis with the
qualification observations: the harness samples the exact deadline clock
immediately before the `sleep-entered` handoff and again after the `wake`
marker, and the owner records the terminal clock sample from the same basis.
The retained basis keeps a truthful post-wake sample available even when the
owner selected terminal immediately after resume. `sleep-entered` is the final
operator handoff immediately before physical suspension (not an OS sleep-entry
event) and is the activeProcess action boundary; `wake` is the first post-resume
operator acknowledgement. Both host-sleep scenarios reuse the private
`/dev/zero -> /dev/null` `DeviceCopy` stimulus with a scoped, one-shot,
`#[cfg(test)]` 120-second qualification deadline; production `DeviceCopy`
remains 300 seconds. The implementation blocker is removed, but the two
physical host-sleep repetitions are still missing until an operator runs them.
It does not add a sleep inhibitor, OS plugin, checkpoint, resume token, or
replay path. The host-sleep deadline phase is anchored to the exact
owned-process deadline-clock start (`DeadlineClockStarted.at`), which is also
serialized as `hostSleep.operationStartedAt`; the earlier
`operation-started` sentinel marker remains an independent chronology
observation and is not the 120-second deadline threshold authority. That wall
timestamp is the wall observation retained alongside construction of the exact
monotonic timer basis, not a later observer-install timestamp. The final
host-sleep lifecycle snapshot is taken only after the bounded watcher has
published its retained-basis post-wake sample, so the owner may reach terminal
immediately after resume and terminal may precede that post-wake sample.
Host-sleep evidence records persist the sanitized owned-process lifecycle
observations in `trace.lifecycle` before deriving `activeProcess` and
`hostSleep`; blocked attempts retain the partial lifecycle trace and the exact
watcher gate error, so a null projection never discards the source
observations.
`transport_loss` requires zero owner-emitted `DeadlineReached` events; the
owner event is the only authority for whether the timeout branch won, and a
monotonic clock sample at or beyond the nominal deadline never converts a
transport failure into a timeout.

Physical interruption qualification is isolated to the ignored Phase 6D.6
harness. It requires one exact selected serial, the committed fixture package
and roots, explicit destructive/root/authorization/identity/host-sleep opt-ins,
an operator-controlled ten-minute sentinel checkpoint, and two clean
repetitions per mandatory scenario. Active interruption requires exact target
child and mutation liveness immediately before the action; a runner callback,
delayed poll, post-operation probe, or harness boolean cannot qualify it. The
current physical adapter emits no exact child evidence and blocks active cases.
Each record has unique run, scope, sentinel, nonce, slot, path, trace, and
canonical content identities, and UI-smoke subcases bind digested UI-state
artifacts to distinct physical backend runs and traces. Host simulations and
deterministic tests are regression evidence only, not physical-device evidence.
Until the complete sanitized matrix and automated verification pass, Phase 6D
remains In progress.

Supported steps include artifact resolution/extraction, file copy, APK install,
launch, force-stop, permission/app-op grants, and waits. Device-target archive
extraction happens in the host staging area before files are pushed through
ADB; Android `unzip` is not required.

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs. The
single-threaded resolver uses strict Rustls certificate and hostname validation,
a 15-second connect timeout, one five-minute transfer deadline, at most five
redirects, and no automatic retries. HTTPS-to-HTTP redirects are rejected.

Cache keys continue to hash the original URL bytes, including query and
fragment. Complete default-cache files are authoritative and bypass URL parsing
and network setup after their artifact type, cache mode, destination, regular
file kind, and sandbox policy pass. Without a valid cache hit, malformed and
unsupported source URLs fail admission. New bytes are streamed to unique
same-directory partial files, flushed, synced, and published without
clobbering. `cache: none` always transfers and uses a unique runtime path on
collision. Failures are typed and redacted, remove partial files when cleanup
succeeds, and block dependent steps without preventing unrelated work.

## Editor

`apps/config-editor` is a React/Tauri application. Tauri builds and packages the
Rust `emuchef` binary as an external sidecar and launches it with `--sidecar`.
The JSONL sidecar owns persistent document sessions and in-memory Phase 0
execution reports/events. Sidecar startup may set `--runtime-root`,
`--cache-root`, and `--adb`; defaults are working-directory-local runtime/cache
directories and `adb`.

Release builds use a local-only production CSP with no Vite development URL.
Frontend sources are limited to the packaged application, while Tauri IPC uses
only `ipc:` and `http://ipc.localhost`. Development-only Vite and HMR settings
live in `tauri.dev.conf.json` and are selected by the maintained Tauri command
wrapper only for `tauri dev`.

Product-facing Tauri commands use non-prefixed document names:
`list_step_specs`, `open_recipe`, `get_document`, `apply_recipe_command`,
`undo`, `redo`, `save_recipe`, `save_recipe_as`, `validate`, `emit_yaml`,
`get_ref_index`, and `set_document_authored_root`. `sidecar_status`,
`sidecar_ping`, and `sidecar_restart` retain the prefix because they manage the
sidecar process. App Definition documents use the corresponding
`open_app_definition`, `apply_app_definition_command`, `save_app_definition`,
and `save_app_definition_as` operations and share document-level inspection,
undo/redo, validation, YAML, authored-root, and close behavior. Transport
request identifiers are removed before responses reach frontend code. The
sidecar also exposes `saveDocument` and `saveDocumentAs`, which dispatch by
document type alongside the App Definition-specific save operations.

Recipe document admission validates the Recipe's own steps and references,
relevant App Definitions, dependencies, and authority graph. Catalog-wide
authored-root validation still reports diagnostics from every catalog entry,
but unrelated malformed entries do not reject an otherwise valid Recipe.
Recipe Save As validates against the destination catalog while publication is
coordinated. Successful publication refreshes the document path, authored root,
diagnostics, and App catalog projection to the destination; rejected admission
does not publish a destination file.

### App Definition documents

App document responses include `owningAuthoredRoot` as an explicit string or
`null`. It identifies the physical catalog that owns the persisted App identity;
standalone documents have no owner. `authoredRoot` identifies the selected catalog
used for browsing and editing context. Selecting another catalog does not change
the owner, identity lock, Save destination, or owner-catalog dependent validation.
Successful canonical Save As publication gives a standalone document its catalog
owner through the persisted identity lifecycle.

App `editRestrictions.canSaveAs` is true only for a standalone document with a
canonical, existing selected root directory whose root/apps membership passes
the existing real-directory restrictions. Missing roots, files used as roots,
and symlinked apps directories disable publication availability and require a
valid catalog selection. This availability signal does not replace strict Save
As admission or alter catalog ownership.

For a standalone App, the Save As dialog defaults to
`<authoredRoot>/apps/<app-id>.yaml` when an authored root is selected. The dialog
selects a destination file, not a catalog root. No selected root requires an
explicit catalog selection before publication, and a destination in another root
is rejected. Existing catalog Apps retain their owner and cannot use Save As to
move or clone their identity.

The sidecar keeps App Definition documents in typed sessions separate from
Recipe documents. App Definition sessions parse the schema-v1 structure and
produce semantic diagnostics independently of strict save admission. A
structurally parseable schema-v1 App Definition with semantic errors opens as an
editable draft. An incompatible kind or unsupported version is rejected with
structured field diagnostics before a session is created; headers are not
automatically converted. Malformed input cannot open as an App Definition document.
Authors can repair one field or collection entry at a time while diagnostics for
other invalid fields remain visible. A malformed command or invalid mutation of
the field it changes is rejected atomically without changing the draft or its
current diagnostics or undo/redo history; candidate rejection diagnostics are
reported separately. Commands are bounded to App Definition fields and named
collections, including app identity and presentation, launcher activity,
artifacts, permission sets, targets, and metadata. Metadata is descriptive and
does not grant authority; reserved top-level metadata keys and duplicate metadata
keys are rejected.

Catalog ownership is inferred from the App Definition's physical `apps/`
location independently of the selected authored-root context. Opening rejects
symlinked `apps/` directories in either the selected authored root or the physical
owner root. Catalog-backed App Definitions retain their canonical ID, package
identity, file path, and
owner root for save admission even when the selected context changes. Saving
updates only the opened canonical file after full admission. A standalone App
Definition may be saved to its opened external file; Save As requires a selected
authored root and publishes it once to `apps/<id>.yaml` there with no-clobber
semantics. The file-dialog destination cannot infer or change that authority.
Other-root destinations, dot-segment paths, and symlink escapes are rejected;
canonical filesystem aliases such as `/var` and `/private/var` are accepted.
A successfully published document becomes catalog-backed; Save As removes
undo/redo history entries incompatible with its newly locked identity. App IDs, artifact
IDs, and target IDs cannot be renamed through local edits; identity changes
that require reference rewriting are not performed by this editor. A different
package identity requires a new App Definition.

Save admission validates the complete App Definition and every directly
affected persisted Recipe in full. It combines shared local validation without
device context, shared App authority validation against the candidate and
relevant other Apps in the snapshot, and bounded static planning. Existing Ref
and StepSpec rules for file outputs, multiple inputs, copy provenance, resolver
ancestry, and required permission policy remain enforced. Errors anywhere in an
affected Recipe block saving, including errors outside the edited app context.
A device API level is not required; limited-context warnings are nonblocking,
while static permission policy is still validated. The affected set includes
typed App Artifact references; step app context, including permission-set and launcher use; and
App Target references in step parameters and conditions. The catalog scan must
classify every Recipe reliably. Unreadable directories or Recipes that cannot
be parsed stop admission with actionable diagnostics instead of being treated
as unrelated. Admission checks relevant App ID and package conflicts, while
known errors in unrelated catalog documents do not block a valid save and source
overlap remains advisory. Admission uses a coherent snapshot of the opened App
Definition, Recipe inventory and affected Recipe files, and competing App
identity files. The backend rechecks those inputs immediately before
publication and rejects stale state detected by that check with a diagnostic.
Publication atomically replaces an admitted file or creates the canonical Save
As destination without overwriting an existing file. Replacement saves preserve
the existing Unix file mode; a detected mode change causes stale-state rejection.
Filesystem publication is not an atomic compare-and-swap operation, so an
external writer racing after the
final recheck is outside the stale-state guarantee. Rejected saves preserve disk
contents and the editable draft. Save failures refresh the active document's
diagnostics panel with file and object locations without changing its content
or history. App Definition diagnostics use normalized object locations across
document operations.

The editor exposes App Definition fields through Overview, Artifacts,
Permissions, Targets, and Metadata sections. Incomplete form input remains
local until the author submits a complete typed change. Diagnostics and
canonical YAML remain visible while an invalid draft is repaired, and Save is
blocked until the App Definition and its directly affected Recipes pass
admission. Toolbar and native-menu Save and Save As actions follow the active
document's effective save availability. Pending App form input counts as dirty
work and disables both actions until it is applied or explicitly discarded.
Undo/redo preserves pending local form fields while refreshing other fields
from the current typed document. If undo removes an artifact or target with
pending input, the form clears its backend identity and keeps the input
available to add as a new entry.
Document-kind-aware menu actions, undo/redo, dirty-state prompts, and recovery
protect work when opening or switching documents, entering a generator, closing
a window, or recovering from a sidecar restart. A canceled or failed open and a
rejected command or save do not replace the current accepted document or discard
its draft. Successfully opening an App Definition exits the user-configuration
view. Ordinary Recipe documents and the existing app and device-profile
generators retain their own workflows; this editor does not migrate legacy
Recipe fields or change runtime authority.

The editor and JSONL protocol also expose catalog-independent user-configuration
open, create, inspect, edit, validate, canonical-emission, save, authored-root,
and close operations. Supplying an authored root adds semantic diagnostics but
is not required to open or emit a structurally valid document. The desktop
editor provides an initial user-configuration surface for selected recipes,
direct binding edits, diagnostics, canonical YAML, and saving; recipe authoring
continues to use its existing document surface.

When a user-configuration document has an authored root, the editor uses
`describeConfiguration` to group inputs by recipe and choose controls from
semantic types. Enums use declared options, booleans use checkboxes, host files
and directories use native pickers, device paths use text entry, and structured
or multiple values use JSON entry. The screen shows required, optional,
advanced, sensitive, and effective-source state without hard-coded recipe or
input identifiers.

## End-User App

The Apple Silicon end-user app implements user-triggered signed update
discovery and manual DMG browser handoff. Rust owns the source-pinned manifest
and DMG URL policies, dedicated metadata public key, no-proxy identity-only HTTP
client, response status/header/length checks, 64 KiB streamed body bound,
canonical fixed-JSON parsing, Ed25519 verification, stable target and version
policy, expiry, retained candidate, activity leases, and operating-system
opener. React receives only display metadata and availability; no URL, key,
signature, raw response, path, or generic opener argument crosses IPC.

Production update trust contains only schema version 1 and `configured: false`.
The local unconfigured status performs no DNS, HTTP, proxy discovery, browser
action, or network-dependent migration. Checks are user-triggered only and
never block startup or ordinary local operation.

The browser downloads the DMG. EmuChef does not inspect or verify the local
file, mount it, install it, replace the app, or restart. Signed size and SHA-256
are release identity metadata rather than proof of the local download. Apple
Developer ID, hardened runtime, notarization, stapling, and Gatekeeper are the
executable trust controls. Saved configurations, recovery, diagnostics, cache,
and managed Platform-Tools remain in app data and outside update authority.

Release tooling reuses Phase 3E credentialed verification, proves that a
read-only release-tooling mount contains the exact verified app tree, emits
deterministic unsigned fixed JSON, and finalizes only after verifying an
external Ed25519 signature. Production trust, hosting, credentialed release
metadata, and clean-Mac manual replacement evidence remain pending. The former
in-place updater remains rejected because its pinned API could not meet the
single-response pre-deserialization trust model.

`apps/emuchef-app` is the separate React/Tauri application for guided
end-user setup. It packages the Rust `emuchef --sidecar` runtime and a bundled
catalog snapshot but shares no frontend modules or runtime state with
`apps/config-editor`. The workflow is Connect Device, Confirm Device, Choose
Setup, Provide Inputs, Review Plan, and Simulated Run. The execution stage is a
fake-device dry run of the exact retained reviewed plan. It performs no real
apply, device writes, artifact resolution, catalog networking, wireless
onboarding, rollback, resume, or parallel-device work, and its result is not
real-device evidence.

The React frontend is keyboard and screen-reader operable across the workflow,
saved setups, execution results, and Troubleshooting. It provides a
skip link, stable landmarks and headings, semantic collections, associated
input diagnostics, focusable error summaries, bounded polite/assertive live
regions, and native determinate execution progress. Status and availability
always have text and structural differences in addition to color. The visual
system uses semantic color, spacing, radius, focus, and status tokens with
system fonts and a dark-only application appearance. Controls, panels, dialogs,
empty states, disabled explanations, long-text wrapping, and result groups use
shared styles. Responsive source and DOM regressions target approximately
380-pixel reflow at 200% zoom, reduced motion, forced colors, focus visibility,
and screen-reader structure without changing the workflow architecture. Those
automated targets do not constitute manual macOS visual or assistive-technology
qualification.

Normal workflow terminology describes the app service, Platform-Tools, setup
catalog, saved setups, installation progress, recovery drafts, and app-owned
cache without exposing implementation language, schema versions, digests,
catalog identifiers, or raw enum values. The header readiness indicator says
only `Ready`; technical identity remains available only through bounded
troubleshooting and diagnostic projections.

Custom prompts and confirmations use exactly-once controllers. Each pending
request has one safe cancellation result, teardown never implies a destructive
or execution-confirming choice, and an overlapping request cannot replace the
live resolver. Runtime restart, configuration replacement, app reset, unmount,
and the top-level frontend error boundary safely cancel pending prompts.
Troubleshooting owns and cancels its nested cleanup confirmation before the
parent closes.

All React modal surfaces use `AccessibleDialog` for focus containment,
deterministic restoration, Escape handling, and the shared dialog-heading and
footer-action placement convention. The Updates dialog disables and explains
its Close action while an update check or browser handoff is active, announces
blocked dismissal attempts, and becomes dismissible again when the unsafe
operation ends.

Modal and native-dialog focus restoration first validates the recorded invoker,
then uses a transition-specific workflow or error destination, the current
workflow fallback, main content, and finally the header Troubleshooting
action. Disconnected, hidden, disabled, or inert invokers are skipped. Focus
never silently falls to the document body, and generation checks prevent stale
restoration from stealing focus from a newer modal or workflow transition.

The app creates, previews, opens, edits, saves, renames, duplicates, imports,
exports, and reuses named portable setup files. Schema V2 contains a generated
configuration identity, presentation name, one authored device-plan reference,
selected recipe IDs, explicitly nonsensitive user bindings, safe additive
extensions, and authored-contract compatibility fingerprints. Fingerprints use
canonical device-plan, recipe dependency/capability/artifact/step/constraint,
input-contract, authored override, profile-capability, and App Definition
authority semantics. App authority semantics include authored step app
references, Recipe-owned App Artifact cache policy, and the package identity
and static source policy of referenced App Definitions and artifacts. They do
not use app presentation fields, resolved release selections, downloaded
bytes, resolved values or artifacts, generated plans, catalog load order, host
paths, device facts, runtime state, review, or execution state. Recipes with
no authored app authority retain their existing fingerprint semantics.
Configuration identity changes do not change fingerprints.

Schema V1 remains readable through an in-memory migration. Because V1 has no
historical contract fingerprints, inspection can establish only whether the
intent validates against the current catalog; it never claims historical
compatibility. The V1 source is not rewritten during inspection or preview.
Its first explicit V2 save establishes the durable compatibility baseline.
Unsupported future schemas and malformed or authority-bearing fields fail
closed without changing the source. Additive V2 `x-*` extensions are preserved.
Other structurally safe unknown fields remain visible as pending sanitation
until an explicit save removes them and reports that consequence.

Tauri owns native setup dialogs, absolute paths, sidecar document identifiers,
opaque document and preview handles, compatibility and comparison decisions,
repair revisions, file writes, and the private ten-entry Recent index. Preview
confirmation rechecks source bytes, source digest, catalog digest, runtime
revision, and preview revision. React receives filename-only context, sanitized
summaries, portable intent, dirty and compatibility states, and friendly repair
actions. It never receives saved-file paths, source document IDs, fingerprint
internals, raw catalog IDs in diagnostics, credentials, or runtime authority.

The native File menu provides New, Open, Open Recent, Save, Save As, Import,
and Export, plus access to the focused Saved setups manager. The manager owns
Rename, Duplicate, missing-file relink, and Remove from Recent equivalents.
Recents use canonical path identity, deterministic last-opened ordering, and a
private path tie-breaker. Distinct paths with the same configuration ID remain
separate and are marked as an identity conflict. Missing entries remain visible
with filename-only context; relinking changes only the private Recent path and
never edits or deletes either file.

Save writes the active identity. Save As writes a new identity and makes that
copy active. Rename rewrites the internal display name and sibling filename
while retaining the configuration ID. Duplicate writes a new identity and adds
it to Recents without changing the active workflow. Import validates a preview,
writes a new-identity copy to an explicit destination, and opens it. Export
writes a sanitized new-identity copy without changing the active workflow or
Recents. Every destination operation is no-clobber. Ordinary writes use a
flushed and synced temporary file followed by an atomic replacement. Rename
with an internal-name change safely creates and syncs the destination before
removing the source; if source removal fails, both valid files remain and the
app reports that state rather than claiming pairwise atomicity.

Opening or importing replaces portable intent and invalidates review and
execution authority. Repair and portable-input relink change intent and also
require fresh validation, planning, and review. Ordinary Save, pure Save As,
and Rename preserve the current workflow stage and review because persistence
identity and presentation name are not plan inputs. Duplicate and Export leave
the active workflow and authority untouched. Repair is explicit and bounded to
exact authored aliases, removal of unavailable optional recipes or retired
bindings, selection of a current valid option, one portable-input relink, and
re-entry of omitted sensitive input. Repair-required setups cannot replace the
active workflow until authoritative validation succeeds. The current authored
recipe schema has no rename-alias contract, so removed recipe IDs are never
matched heuristically; a future exact alias can be applied only after that
authority is defined in the catalog schema.

Opening a setup classifies bindings through the authoritative
`describeConfiguration` operation. React receives only active bindings marked
explicitly nonsensitive. Sensitive, inactive, and unclassified bindings stay
backend-owned and appear only as a pending-sanitation count or label-based
re-entry requirement. Opening, previewing, cancelling, or closing never
rewrites the source. Explicit Save and Save As remove omitted bindings before
writing, so credentials and resolved sensitive values are never persisted or
exported.

Phase 2B guarded real-device execution is implemented behind the
default-disabled Cargo feature `real-execution`. Rust projects the immutable
`ExecutionCapabilities` contract containing `realExecutionCompiled`,
`platformToolsStatus`, and `executorReadiness`. Ordinary development and
production commands report real execution as not compiled, Platform-Tools as
not applicable, and the executor as not compiled; they remain simulation-only
and do not perform readiness validation. The development-only
`tauri:dev:real` npm command intentionally passes `--features real-execution`
through the Tauri CLI and reports real execution as compiled; it does not alter
Cargo defaults or any build, bundle, release, or packaging command.

## Developer commands

The repository root `Makefile` provides the standard developer workflow.
`make build` builds the Rust backend and both frontend codebases. `make test`
runs the full automated Rust, application, security, typecheck, and lint suite.
`make emuchef-app` launches the end-user app, `make config-editor` launches the
Config Editor, and `make dev` launches both apps concurrently. These Makefile
targets use ordinary simulation-only development commands; they intentionally
do not use the separate real-device `tauri:dev:real` command.

`make clean-preview` validates the invoking Git worktree and reports the
eligible and skipped cleanup entries with approximate sizes without changing
the filesystem. `make clean` removes only recognized, untracked Cargo build and
test intermediates under the three application/backend target roots and the
Tauri-generated `apps/*/src-tauri/gen/schemas/desktop-schema.json` files. Cargo
cleanup is restricted to recognized files below `.fingerprint`, `build`,
`deps`, and `incremental` in `debug` and `release` profiles. Cross-target
profiles are considered only when the target-directory name has the canonical
Rust target shape and uses a recognized standard architecture and vendor.
Unknown Cargo-shaped directories remain in place and are reported as skipped.

Default cleanup preserves the target roots, every `target/**/bundle/**` tree,
the backend freshness `.pending` marker, tracked files, and unknown files. It
also preserves installed dependency trees, frontend `dist` output, prepared
Tauri sidecars and metadata, `.emuchef_cache`, `.emuchef_runtime`, release
artifacts, documentation evidence, snapshots, fixtures, authored content, other
worktrees, and editor or agent indexes. Default cleanup may remove the obsolete
backend freshness digest;
`crates/emuchef-rust-backend/target/.emuchef-cargo-test-source.sha256.pending`
continues to invalidate stale backend test results until `backend-test-fresh`
recreates the digest safely.

`make clean-deep` is a separate opt-in action. It prints a warning and a
preview before removing the default eligible set plus untracked regular files
under both applications' `node_modules` trees, their dependency-install
stamps, recognized frontend `dist` files, prepared Tauri sidecars, and their
metadata. The next dependency or build command may therefore reinstall
packages, rebuild frontend output, or regenerate a sidecar. Tracked descendants,
nested `.git` metadata, unknown frontend or binary-directory entries, symlinks,
runtime and qualification data, and release or evidence trees remain preserved.

The cleanup and both sidecar producers import the same target contract. It
accepts canonical Rust host-target tuples, requires `.exe` for Windows targets,
requires no executable suffix for other targets, and preserves metadata and
near-match names that do not satisfy both rules. Both producers continue to
obtain their default target dynamically from `rustc --print host-tuple`.

Cleanup assumes an idle operator: builds, tests, development servers, and
packaging must not be running when any cleanup action starts. The tools use
ordinary path and symlink checks and do not add process discovery, locks, or
coordination for active work.

The checked-in `EmuChef execution feature matrix` GitHub Actions workflow is the
continuous compile-policy authority for Phase 6A. It runs `cargo check` and
`cargo test` for the Tauri crate with `--no-default-features` and with
`--no-default-features --features real-execution`, using the committed lockfile.
A separate workflow job runs the source-level security policy suite, which
proves that Cargo defaults and ordinary development, packaging, and release
scripts remain feature-disabled and that only `tauri:dev:real` enables the
feature. The workflow does not build or publish a production artifact with real
execution enabled.

Feature-enabled capability refresh clones the resolved managed or development
ADB state and its revision while holding the trusted mutex, releases the mutex,
then performs bounded local signature, retained-file, and `adb version`
validation without device enumeration, server startup, or device commands. A
result is published only if both the ADB revision and runtime generation still
match. Platform-Tools reports ready, not found, invalid, or check failed, while
executor readiness reports ready, blocked, or unknown. Infrastructure failures
surface only the sanitized command error
`execution_capabilities_unavailable`; tooling outcomes remain typed capability
states.

Capability refresh runs at startup, after app-service restart, and after
managed Platform-Tools install, replacement, or removal. While a lifecycle
refresh is active, React retains the prior valid capability value and marks the
diagnostic rows as refreshing. A failed refresh retains that value and reports
status unavailable rather than manufacturing a new diagnostic. Capability
readiness is informational: it does not change real-execution visibility,
eligibility, confirmation, mode selection, or start authority. Guarded real
execution independently revalidates Platform-Tools immediately before listing
devices or starting execution. Trusted Tauri code owns confirmation, mode
selection, exact target identity, reviewed plans and digests, Platform-Tools
revalidation, unambiguous retained BYO file/directory checks, and sidecar
identifiers. Platform-specific packaged-device evidence, privacy and security
approval, an operator runbook, and a separate release decision are required
before a release build opts into the feature.

Tauri launches the sidecar and negotiates the `phase0_end_user_runtime`
extension plus `describeCatalog`, `listAdbDevices`, `probeDevice`,
`matchDevice`, `describeConfiguration`, `planConfiguration`, `startExecution`,
`getExecution`, `getExecutionEvents`, `cancelExecution`, and the schema-v1
user-configuration document operations before ADB is needed. Runtime and
catalog startup are independent from Platform-Tools setup.
A missing ADB installation blocks only device discovery and displays the
Platform-Tools setup flow.

The end-user app's maintained macOS packaging target is a thin Apple Silicon
`aarch64-apple-darwin` release bundle with a macOS 11.0 minimum. It bundles the
release Rust sidecar, authored catalog snapshot, and checked-in qualification
policy through Tauri resources. Intel and universal end-user bundles are not
qualified. Platform-Tools and user content remain external.

The checked-in `app-icon.png` is the byte-preserved approved branding master.
Tauri packages a generated 512-by-512 `icon.png` and a multi-resolution
`icon.icns` derived from that master without cropping, stretching, padding,
background insertion, or redesign. The native About menu uses the package name
and version from the running Tauri application's package metadata and includes
the product description and GNU GPL v3.0 credit; frontend and menu source do
not duplicate the configured version.

Local macOS qualification is ad-hoc, requires no private Apple credentials,
removes the fixed Apple credential allowlist from its child environment, and
explicitly sets the ad-hoc signing identity. It statically verifies product
metadata, thin arm64 executables, nested and deep signatures, resources,
catalog content, production CSP, minimal capabilities, path independence, and
default-disabled real execution. A copied-app probe uses a canonical temporary
root and temporary home/data/cache locations to negotiate the packaged sidecar,
load the bundled catalog, and perform a read-only catalog operation without
ADB, network access, a device, Platform-Tools, BIOS files, or ROMs.

Credentialed developer-id mode is an explicit separate operation. Only that
mode validates the fixed supported Apple variable allowlist; values stay in
the caller environment or keychain and are not printed or serialized. Local
qualification cannot establish Developer ID identity, notarization, stapling,
Gatekeeper acceptance, clean-Mac behavior, or public release readiness.

End-user macOS manifests record toolchain versions, target, release mode,
source commit, tracked dirty state, qualification-policy version, normalized
security-relevant content, and raw per-build artifact identities. Normalized
content covers signature-removed executable bytes, Info.plist semantics,
catalog resources, capability policy, Tauri security configuration, sidecar
identity, and default-disabled real execution. Code signatures, timestamps,
DMG container metadata, mtimes, temporary names, and caller-specific absolute
paths are excluded or normalized. Raw app and DMG hashes identify one build
and are not claimed to remain byte-identical across rebuilds.

Opening or reusing a portable configuration invalidates every prior generated
plan, digest, review, execution, confirmation, launch action, target binding,
and probed device fact. The user must select and freshly probe a current device,
validate against the current catalog and device capabilities, generate a fresh
description and plan, and complete a fresh review before execution. Stale plan
references, recipes, and bindings remain visible for correction and are never
silently substituted.

Save/Discard/Cancel protection applies before actions that would lose, replace,
close, reload, or invalidate dirty portable edits. Platform-Tools removal keeps
the active document and dirty edits intact while invalidating device, review,
execution, confirmation, and launch authority, so removal itself does not
require the dirty prompt.

The end-user app keeps at most one schema-v1 recovery record for dirty portable
intent under a Tauri-owned fixed application-data path. The record is bounded,
atomically replaced, owner-only on Unix, and generation checked. It can contain
a safe display name, authored device-plan reference, recipe IDs,
schema-permitted bindings, key-only omitted-binding markers, and a source
saved-configuration identity. It never contains device, generated-plan,
review, execution, confirmation, launch, dialog, cache, diagnostics, sidecar,
log, process-output, or raw-error authority.

Startup validates the recovery record before normal workflow use and offers
Restore, Discard, or Not now. Restore creates fresh document/frontend sessions
and requires current device selection, probing, validation, description,
planning, and review. Not now keeps the exact generation on disk across clean
shutdown and offers it on the next launch. Deferred disposition is separate
from current-session dirty state. Explicit discard, successful Save or Save As,
or a newer atomically staged dirty generation clears or supersedes it.

Only authored input `sensitive` metadata controls binding recovery. Explicitly
non-sensitive values follow the existing portable saved-configuration rules,
including existing file/directory bindings. Sensitive and unclassified values
are omitted without hashes, masks, or length leakage and produce a sanitized
required-re-entry diagnostic after restore. Key names and value shapes never
classify secrecy.

## End-User Troubleshooting and Local State

EmuChef proper exposes one Rust/Tauri-authored troubleshooting projection for
the app service, Platform-Tools, connected devices, the bundled catalog,
app-owned cache, updates, saved-setup and recovery state, and retained
execution status. The projection contains only the subsystem label, bounded
severity, plain-language summary and consequence, optional stable public
support code, and closed corrective-action variants needed by the support
surface. Healthy state is summarized once, affected subsystems receive primary
emphasis, and neutral optional or unconfigured capabilities do not receive a
failure code. The projection is not an independent application state machine.

Public support codes are fixed uppercase ASCII constants in a closed Rust
registry. Each code has one subsystem and bounded meaning; dynamic internal
error text, internal result names, paths, serials, command lines, logs, stack
traces, credentials, configuration contents, input values, plans, and runtime
authority never enter the code or normal troubleshooting DTO. Unknown internal
failures map to a deterministic bounded fallback without projecting their
text. React exhaustively routes known corrective-action variants and fails
closed for unknown variants.

Corrective actions use the narrow authority revision owned by the affected
subsystem. App-service restart uses the service generation, managed
Platform-Tools changes use the Platform-Tools revision, device refresh uses the
device generation, and cache deletion uses its inventory generation and opaque
entry handles. Destructive work is revalidated immediately before mutation.
Cache cleanup is restricted to fingerprint-matching direct children of the
canonical app-owned cache root that remain managed, removable, in an approved
category, and not in use. Missing approved entries are idempotent; changed,
external, unmanaged, protected, or in-use entries are not deleted. Replace and
Remove Platform-Tools are offered only for the app-managed installation and
never delete a user-selected, PATH-resolved, system, or external installation.

Support and diagnostics outcomes are scoped to their modal and operation
generations. Opening or closing Troubleshooting, refreshing inventory, or
starting a retry clears superseded presentation notices, and stale completions
cannot replace current state. Diagnostics export creates a local ZIP no larger
than 2 MiB and never uploads it. Schema version 2 contains only the fixed
members `manifest.json`, `runtime.json`, `catalog.json`,
`configuration-summary.json`, `execution-summaries.json`,
`cache-summary.json`, and `support-status.json`; the archive contains bounded
aggregate state and active public support codes, not support UI state, action
or reset handles, presentation generations, historical notices, raw errors,
logs, paths, serials, credentials, configuration bodies, input values, or
plans.

Reset Local App State presents only categories that have current app-owned
state and issues one-shot opaque reset handles bound to that category's
revision. Current categories clear the Recent setup index, approved app-owned
cache entries, or the recovery draft. Each has separate description, scope,
consequence, availability, and explicit confirmation. Clearing Recents does
not close an active document or delete saved setup files. Cache reset does not
delete configurations or external content. Recovery reset is distinct from
Restore and Discard workflow decisions and does not clear the live-process
marker, active workflow, saved files, portable intent, review authority, or
external content.

The recovery active-session marker represents process lifetime, not window
lifetime. Closing one window or the final macOS window, cancelling a close, or
restarting the local app service leaves the marker active while the process is
alive. Only an accepted application exit, including Cmd+Q, or an
application-controlled exit/relaunch finalizes it synchronously before process
termination. A crash or other unclean termination leaves the marker for the
next launch, preserving genuine interruption detection. Once an accepted
termination finalizes the marker, qualification lifecycle work that has not
already committed fails closed instead of writing durable session state that
the next launch would treat as part of a clean handoff.

Packaged catalog data is materialized under the application resource directory.
The trusted backend requires the four product directories, ignores only regular
`.gitkeep` placeholders, rejects symlinks and every other unsupported entry,
computes the canonical catalog SHA-256, and passes a
resolved snapshot to the sidecar. React receives source identity, version, and
digest without a catalog filesystem path. The catalog remains a bundled MVP
source; Phase 4B release discovery does not update or replace catalog content.

Exact ADB serials and executable paths exist only inside trusted Rust/Tauri
communication. Tauri assigns stable opaque device handles while a serial
remains present during one application session and invalidates them on device
disappearance. React receives the opaque handle, masked serial, display facts,
connection state, and actionable errors. The production device probe also
preserves the optional Android build fingerprint from `ro.build.fingerprint`
as an internal `firmware_build` fact and exposes it at the React boundary as
nullable `firmwareBuild`. Exact serials never enter React payloads, state,
logs, storage, or markup.

The backend owns exact/high/low/none device matching. A unique exact/high match
may recommend a plan. Low/no matches are never auto-selected. Backend-approved
safe generic plans may be offered for explicit user choice, and the workflow
blocks only when the backend reports no safe candidate.

`describeConfiguration` is authoritative for recommended/default, optional,
dependency-required, and unavailable recipes; effective dependency expansion;
input declarations, values, provenance, and diagnostics; and review readiness.
React does not reconstruct planner rules. Native Tauri dialogs supply host file
and directory recipe inputs without blocking the IPC executor. Single-file,
multi-file, and directory cancellation leaves the input unchanged. Input-level
diagnostics render only with their field; aggregate diagnostics remain at page
level only when no input diagnostic represents the same binding key and code,
or the same code and message when no binding key is available. React and Tauri
send camelCase product fields;
probe facts remain snake_case inside the trusted inventory DTO and are converted
to camelCase `deviceContext` and `targetDevice` objects for the sidecar. The
selected-device command and qualification orchestration share the trusted
production probe helper; neither path issues a second build-fingerprint probe.
`selectedRecipes: null` selects device-plan defaults, while an explicit empty
array selects no recipes. Missing required input values remain successful
description results with `binding_missing` diagnostics instead of transport
failures.

Each accepted configuration description replaces the runtime session's current
native input-contract snapshot. A request sequence rejects out-of-order
descriptions without creating a separate authority generation. Tauri derives
picker kind, multiplicity, extension filters, and immediate filesystem checks
from that snapshot; React supplies only the input key and portable edit intent.
Device paths never receive host-picker authority.

The Inputs stage presents authored labels and descriptions, required or
optional state, single or multiple value state, accepted extensions, and safe
authoritative constraints. Untouched missing requirements remain neutral until
the user edits the field or explicitly requests validation or review. Host path
validation detects missing, inaccessible, wrong-kind, unsupported-extension,
and canonically duplicate entries before review. Canonical duplication inside
one input is blocking; reuse across active inputs is a non-blocking warning that
names both labels. Errors, logs, omission notices, diagnostics, and support
surfaces do not include host paths, while portable user-selected paths remain
visible in the ordinary input UI as selected intent.

Multi-file inputs expose per-entry validation and support add, replace, relink,
remove, and clear. Relinking replaces only the selected entry and retains the
device plan, recipe selection, and unrelated bindings. Missing or inaccessible
retained paths are repaired by explicit user selection; EmuChef never searches
the filesystem heuristically. Drag-and-drop is not part of the current input
workflow.

The trusted configuration-description response retains the exact target device
binding and verified catalog identity/digest. The React projection omits the
target binding, catalog root, raw sidecar payload, and all exact serials.
Sidecar failures map to stable sanitized configuration error codes and useful
recovery messages. Debug builds log the complete internal error to the Rust
terminal only after redacting exact serials and absolute paths; release builds
do not expose raw internal errors.

Tauri retains the complete immutable reviewed-plan result, exact target binding,
catalog identity and digest, and canonical plan digest behind an opaque review
handle. Rust owns the review meaning and classification. Tauri only retains the
exact plan, verifies trusted identity and digest state, attaches the opaque
handle to the backend-authored projection, and defensively redacts the exact
serial. React renders the projection without receiving the plan digest, recipe
or step IDs, input keys, capability tokens, raw parameters, hashes, host paths,
or diagnostic codes. A review marked unsafe by the backend cannot start either
simulation or real execution. At most 16 live reviews
are kept in memory, with a 30-minute idle lifetime, two-hour absolute lifetime,
and 64 bounded tombstones. Device disappearance, changed device or
qualification facts, session changes, Platform-Tools replacement/removal,
discard, or capacity eviction returns
`review_stale`; time expiry returns `review_expired`; an unrecognized handle
returns `review_unknown`.

Simulation start accepts only the opaque review handle at the React boundary.
Tauri revalidates review lifetime, connected-device presence, retained target
facts, and device, qualification, and session freshness. It verifies the exact
retained plan against its canonical digest without repeating profile matching
or planning. Catalog identity and digest remain planning-time provenance;
catalog edits alone do not invalidate an unchanged reviewed plan. Fresh
planning captures catalog presentation edits and produces a new plan digest.
Tauri then sends the exact retained plan, digest, and target to `startExecution` with
trusted code forcing `dry_run`. A device disconnect blocks start but cannot
cancel or erase a dry run after it has started.

Random execution handles are session-scoped, never reused, and lost on restart.
The shared simulated/real store retains one kind-aware start reservation or
active mapping and at most the latest terminal mapping. Every failed preflight
or rejected start releases the reservation; a public handle is bound only after
the sidecar returns a successful execution identifier. A successful start
response that cannot establish that identity — a missing execution object or an
empty execution identifier — proves the runtime accepted work the app cannot
identify, poll, cancel, or bind, so Tauri fails and stops that runtime session,
clears the authority derived from it, and reports `runtime_session_lost`
instead of an ordinary start failure. Wrong-kind lookups are
indistinguishable from unknown handles. A lost real sidecar session removes
only the matching active or latest-terminal mapping, invalidates its originating
review, and reports an unknown outcome without inferring terminal status.
Reviews otherwise retain their independent stale, expiry, discard, and capacity
lifecycle.

A device-inventory refresh can be the first native request to discover that the
sidecar generation is gone. That discovery discards the authority derived from
the lost generation — retained executions, device facts, root evidence,
reviews, and any active qualification attempt — and reports
`runtime_session_lost` instead of an inventory failure. A frontend request that
receives this code resets the stale workflow projections and re-reads the
runtime, Platform-Tools, and qualification projections, so the app-service
recovery controls are offered again. Simulated execution snapshot and event
polling classify a proven lost runtime session the same way, so the centralized
runtime-loss recovery runs instead of an execution-local unavailable
transition; a genuinely unknown simulated execution remains a mapping-local
`execution_unavailable` that leaves other authority intact.

When a terminal execution snapshot is retained, the trusted store also retains
the production report and runtime metadata needed for report capture. The
`production_execution_report_bytes` helper owns the sanitized projection,
redaction, deterministic pretty-JSON serialization, and trailing newline used
by both the native export dialog and qualification evidence capture.
An eligible configured-app launch action is consumed before external
revalidation and is never reusable. A failed attempt retains one fresh opaque
action immediately when the retained terminal report still authorizes that
launch; successful launches, lost executions, and executions that are no
longer eligible expose no replacement. Execution reads only project whichever
action is already retained.

`getExecution` snapshots are authoritative for feature-grouped progress,
current user-facing action, completion counts, and terminal state. Incremental
events are presentation data only and use their own accepted-event cursor;
snapshot sequence values do not skip unseen events. Snapshot and event polling
share generation and opaque-handle guards, and a terminal state cannot be
downgraded by a late active snapshot. Polling stops after the terminal snapshot
and its concurrently retrieved event batch are accepted. Visible timestamps
are localized by React while canonical timestamps remain in retained and
exported reports. Export-success presentation is scoped to the execution
generation and handle, not to changing sequence values.
Cancellation is cooperative: completed steps remain visible, no new work starts
after cancellation is observed, and the current atomic operation may finish.
Real execution provides no rollback or restoration. New real projections are
serial-free and allowlist messages, target facts, and report fields; Android
version is omitted unless an existing trusted string supplies it and is never
derived from API level. User-facing issues resolve opaque executor identity
inside trusted code to authored feature and action text, pair it with
backend-classified recovery guidance, and omit raw verifier/dependency codes.
Failed, cancelled, unavailable, stale, and repaired runs require a fresh plan
and review before execution; a prior review may remain visible only as
non-executable history. React projections omit the sidecar execution id, full
plan, target binding, catalog root, step outputs, arbitrary paths, and raw
sidecar errors.

EmuChef never bundles, vendors, redistributes, mirrors, proxies, or downloads
Android SDK Platform-Tools. The setup UI opens only Google's official
Platform-Tools page and imports a macOS ZIP through a backend-owned native
picker. React cannot submit an archive path. Validation limits archive size and
expansion, rejects encryption, traversal, symlinks, special entries, and
case-colliding names, and extracts only `adb`, `NOTICE.txt`, and
`source.properties` to private application data. The backend records and
rechecks SHA-256 for all three retained files, native Mach-O compatibility,
Google signer Team Identifier `EQHXZ8M8AV`, supported version, and controlled
`adb version` execution.

Platform-Tools 35.0.0 is the minimum supported release and 37.0.0 is the tested
upper bound. Valid newer releases are accepted with an untested-version warning.
Release builds resolve only the validated managed installation and never depend
on `PATH`. Debug builds may use an explicit `EMUCHEF_ADB_PATH` override first or
deliberately enable system ADB lookup with `EMUCHEF_ALLOW_SYSTEM_ADB=1` after
managed lookup. Failed replacements preserve the prior active installation and
settings; removal and cleanup are restricted to the managed application-data
root.

## Testing and Compatibility

Rust unit and integration tests are the product behavior authority. Frontend
typechecking, linting, logic tests, and Tauri tests cover editor behavior and
sidecar packaging.
macOS packaging automation inspects a caller-supplied `.app` rather than a
developer-specific path. It verifies Info.plist identity, host architecture,
the main executable and bundled `emuchef` sidecar, embedded frontend markers,
local signing state, dynamic dependencies, and the absence of Python, shadow,
legacy, and development-server remnants. Separate smoke commands exercise
direct JSONL hello/ping and application launch against that exact bundle.

The network CLI integration test can target the exact packaged `emuchef`
executable through the test-only `EMUCHEF_TEST_BINARY` environment variable.
This test seam is not read by product code. It proves local HTTP cold, warm,
and offline cache behavior plus self-signed HTTPS rejection without changing
the macOS trust store.

`crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1` contains
frozen v1 compatibility fixtures. They are immutable evidence, are not
regenerated from Python, and may change only through an explicit compatibility
contract decision. New behavior uses Rust-native fixtures and tests.

Normal automated verification does not run real-device apply. Device evidence
is collected manually using
`docs/manual/real-device-retroarch-validation.md`. The evidence record at
`docs/release/evidence/real-device-retroarch-2026-07-11.md` confirms a successful
local-artifact baseline, real-device apply and idempotent rerun, clean-cache
HTTP(S) resolution, warm-cache and offline warm-cache reruns, matching cache
manifests, and no leaked partial files on commit
`5dca50603cf3a4831867c229157a94906151cbb7`.

### ARMSX1 emulator qualification target

The operator procedure for live ARMSX1 qualification requires serial
`emulator-5554` and AVD `EmuChef-ARMSX1-Android16-ARM64`. Diagnostic mode
verifies that exact serial-to-AVD identity using read-only ADB observations.
Normal Node live invocation validates the selected AVD identity and accepts
any syntactically valid emulator serial. Before mutation-capable work, the
Rust live qualification test independently requires `emulator-5554` and
rejects any other serial. The reviewed install and package-edit target binding
uses that same serial. Neither workflow enumerates attached devices or proves
that `emulator-5554` is the only running emulator. Offline qualification and
live emulator qualification remain separate evidence paths.

Before the ARMSX1 APK install operation reaches ADB, the Rust qualification test
verifies the downloaded digest, package identity, ARM64 native library, and APK
signature. The `apksigner` verifier must be available on `PATH` or under an
Android SDK build-tools directory found through `ANDROID_HOME` or
`ANDROID_SDK_ROOT`. Missing verifier, verifier execution failure, and APK
signature rejection use the stable codes `apk_preinstall_signature_verifier_unavailable`,
`apk_preinstall_signature_verifier_failed`, and
`apk_preinstall_signature_rejected`. These failures belong to APK identity
verification; the install and version stages remain `NOT_RUN`, and package,
ABI, and signing evidence reflects only completed checks. The
`android_signing_failed` code and `install_signing` category describe a
signing rejection returned by Android after the package-manager install
command was issued.

## Release State

Host-target Tauri builds and simulated bundled-sidecar tests are available.
The canonical operator procedure for macOS release bundles is
`docs/manual/macos-packaged-gui-validation.md`; it separates static bundle,
packaged sidecar, packaged runtime, and interactive editor evidence. The editor
exposes side-effect-free planning, but not apply, through its Tauri and JSONL
command surfaces.
The **current EmuChef proper Apple Silicon macOS** release remains blocked until its exact-build packaged GUI/sidecar and macOS artifact inspection, Developer ID/hardened runtime, notarization/stapling/Gatekeeper, clean-Mac behavior, production update trust/hosting and signed metadata, qualified supported device/workflow evidence and explicit release authority are established. The Config Editor's separate release remains deferred; historical editor artifacts/checklists do not satisfy current EmuChef proper evidence. Other host platforms and their cross-platform release automation are separately scoped, **not unconditional blockers** of the initial Apple Silicon EmuChef proper release. See `docs/release/release-readiness.md` and `docs/product/qualification-obligations.md`.

The macOS `0.1.0` application and disk image built from commit
`93f816fc1ea59cd034a40432e4e2a269e11eead7` have completed Developer ID signing,
hardened-runtime validation, Apple notarization, ticket stapling, local
Gatekeeper assessment, and installed-application assessment. Sensitive Apple
identifiers and raw notarization output remain external. This is local-Mac
evidence; separate clean-Mac validation remains pending.

Release tooling validates the Apple environment without printing credential
values, verifies Developer ID signatures, Gatekeeper notarization, and stapled
tickets independently, and generates a path-safe SHA-256 manifest. The
maintained macOS release verification command supports exact Tauri artifact
discovery or explicit artifact paths, runs the existing bundle and packaged
runtime smokes, and never submits, staples, uploads, or publishes artifacts.

## Phase 6B device qualification contract

Phase 6B device qualification is a Rust-owned projection keyed by an opaque
session device handle. React receives no resolved ADB serial and uses only the
opaque handle, runtime generation, and qualification revision for stale-result
guards. Passive qualification distinguishes feature-disabled, absent,
unauthorized, offline, ambiguous, unsupported, incompletely qualified, and
supported states using sanitized normalized facts; multiple devices never
produce an implicitly selected target.

Root qualification is an explicit user action. The native command resolves the
selected handle to its current trusted serial and asks the sidecar to run the
bounded probe `adb -s <serial> shell su -c id` with a 30-second timeout. Its
sanitized result is one of `granted`, `denied`, `unavailable`, or
`checkFailed` with `timedOut`, `transport`, or `unexpectedResponse`. Only
recognized ADB/device transport failures use the `transport` reason; completed
shell failures use the unavailable, denied, or unexpected-response rules.
Results are held in an in-memory native store bound to the handle, runtime
generation, and Platform-Tools revision. A root result change invalidates
reviews and capability-dependent configuration projections. Polling removes
cached evidence whose device, runtime, availability, or Platform-Tools context
no longer matches; the removed opaque handle is used to stale only reviews
planned with that evidence. A failed prerequisite is resolved before a root
check is reserved, so it cannot leave a check permanently in flight.

Real execution performs one lazy root preflight before any root-capable step,
app-private skip check, app-private read/write, or app-private verification.
The outcome is cached for the run; a failed preflight aborts later steps before
any privileged operation. Ordinary simulation and ordinary development/build
scripts remain unaffected. Automated tests cover the protocol, projection,
classification, single-flight/stale behavior, capability masking, and
executor guard. Physical-device, packaged-GUI, signing, notarization, and
release qualification are not implied by those tests.

Host input binding validation reports a missing-path diagnostic only when host
metadata cannot be read; an existing path of the wrong kind reports a kind
mismatch, and an existing path of the expected kind is accepted. This keeps
configuration planning usable for the root-capability review gate.

## Device qualification repository contract

The repository-owned device qualification Node tool remains the canonical
authority for target registration validation, qualification-candidate
promotion, evidence-bundle validation, canonical digests, compatibility
projection, current-evidence selection, and matrix rendering. Production
runtime behavior is unchanged by these repository contracts.

Trusted Rust candidate persistence uses the compile-time repository root and
the ignored `.emuchef_runtime/qualification-candidates/` directory. Each
candidate is stored beneath an opaque
`qualification-candidate-<32 lowercase hex>` handle. Its `candidate.json` is a
strict Rust-owned envelope containing the handle, candidate kind, captured-at
metadata, optional typed build identity, opaque Node-owned semantic payload,
and optional report metadata. A declared report metadata entry names only
`execution-report.json` and binds its byte length and SHA-256 to the exact
`execution-report.json` bytes in the same directory. Candidate creation stages
both files in a private temporary directory, synchronizes them, and publishes
the complete directory with create-new collision behavior. Restart recovery,
read, discard, and promotion reject symlinked root components, candidate
directories, candidate files, and report files; no candidate path can escape
the fixed root. Rust checks only this local envelope, file, and build-binding
integrity and does not reimplement Node semantic validation. The Rust boundary
accepts only the fixed `--describe`, `--register-target <handle>`, and
`--record-run <handle>` operations. Promotion is refused before Node is
invoked when build identity is missing or stale, and operation results use a
strict Rust-owned envelope around an opaque canonical payload. React receives
no repository path, candidate path, executable path, or arbitrary process
arguments.

Qualification-run candidates created when a session starts are provisional and
remain non-promotable until terminal finalization writes a supported
`runValidity`/`qualificationOutcome` classification. The Rust `record_run`
boundary rejects those candidates before invoking Node; a terminal
`invalid`/`not_observed` candidate remains eligible for canonical audit
recording.

`docs/testing/device-qualification/device-targets.json` now uses schema
version 2. Every material target fact is stored as an exact `{ value, source }`
wrapper, with legal provenance restricted by field: ordinary identity facts use
`production_observation`, `rootState` uses `explicit_root_check`, and
`connectionType` may use `operator_attestation` or
`production_observation`. Registered target IDs are deterministic
`device-target-sha256:<64 hex>` digests over the wrapped facts' values only, so
policy arrays and provenance-source changes do not alter identity while
material fact-value changes do.

Device qualification evidence records and fingerprints also use schema version
2. Embedded `deviceTarget` facts preserve the same typed provenance wrappers as
the registered target, while compatibility fingerprints project only the fact
values needed by workflow invalidation. `fingerprint.emuchefBuild` is a strict
object containing the application version, exact Git commit, canonical
material-build digest, `realExecutionEnabled`, and
`qualificationContract`. Compatibility for the `emuchef_build` dimension
compares the application version, material-build digest, `realExecutionEnabled`,
and qualification contract only; the exact Git commit is preserved as audit
provenance but does not invalidate evidence by itself. The material-build
digest is repository-owned and covers the current working tree contents of the
tracked product/runtime/authored inputs under `authored/`,
`crates/emuchef-rust-backend/`, and the application source roots plus the exact
package/lock/config files declared by `tools/device-qualification.mjs`, while
excluding qualification-only UI/runtime files and evidence artifacts.

Physical-device evidence is now bundle-shaped. Each recorded run directory under
`docs/testing/device-qualification/evidence/` contains `evidence.json` and,
when report capture succeeded, a digest-bound `execution-report.json`. Every
valid run must contain exactly one `artifacts` entry for
`execution-report.json`, the matching required automated observation, and
report bytes whose SHA-256 matches the bound artifact. Invalid
`qualificationOutcome: "not_observed"` audit bundles may omit the artifact and
report file only when report capture itself was unavailable. Synthetic evidence
fixtures under `tests/fixtures/device-qualification/` follow the same bundle
layout.

The canonical workflow contract for `retroarch-plus-bios` is now version 2.
Its prerequisite `clean_or_deliberately_reset_device` remains declared in
`prerequisites` and is also a required human checkpoint with allowed outcomes
`pass`, `fail`, and `unable_to_verify`. This checkpoint is a pre-execution
qualification gate: only `pass` may participate in a valid record, while
`fail` or `unable_to_verify` force an invalid `not_observed` run rather than a
product qualification failure.

Qualification promotion is create-new and repository-bounded. Target and run
promotion accept only `qualification-candidate-<32 lowercase hex>` identifiers
through `node tools/device-qualification.mjs --register-target <candidate-id>`
and `node tools/device-qualification.mjs --record-run <candidate-id>`. Before
any canonical mutation the tool rechecks that the candidate build commit still
matches `HEAD`, the tracked worktree is clean, the candidate material-build
digest matches the current repository state, `qualificationContract` still
matches the current tool contract, and `realExecutionEnabled` remains true. Run
promotion also reloads the current workflow catalog, target registry, authored
recipe digests, and runtime contract, rebuilds the expected fingerprint, binds
the embedded evidence `deviceTarget` from the registered target rather than the
candidate payload, and rejects any drift before sealing the run.

`tools/device-qualification.mjs` is also the sole repository authority for
deriving build/runtime identity. `node tools/device-qualification.mjs
--build-identity` prints the canonical current build identity, optionally with
`--require-clean` to reject tracked worktree drift. `node
tools/device-qualification.mjs --describe` prints a machine-readable repository
description containing schema version 1, runtime contract
`real-execution-v1`, qualification contract version 1, the canonical build
identity, the validated workflow catalog, and the validated device-target
registry. Active repository projection and validation no longer read
`EMUCHEF_PHASE_6F_BUILD_IDENTITY` or `EMUCHEF_PHASE_6F_RUNTIME_CONTRACT`.

Recordable qualification application builds use the pinned
`apps/emuchef-app/scripts/run-device-qualification.mjs` launcher. It performs
the frontend build, prepares the development Rust sidecar, and then starts the
Tauri application with the `real-execution` Cargo feature and
`EMUCHEF_DEVICE_QUALIFICATION=1`; it does not use Vite hot reload or `tauri
dev`. When that opt-in is present, the Tauri build script obtains the build
identity only from `tools/device-qualification.mjs --build-identity
--require-clean` and embeds the resulting strict camelCase JSON as
`EMUCHEF_QUALIFICATION_BUILD_IDENTITY`. Rust qualification mode requires a
debug build, the compiled `real-execution` feature, the explicit runtime opt-in,
and a successfully parsed embedded identity whose real-execution capability is
enabled. Before invoking the canonical Node command, an opted-in Cargo build
watches that tool, the declared authored/backend/application material roots and
exact package/lock/config files, every tracked repository path, and the Git
reference/index state used by the identity command, so a changed input cannot
reuse stale embedded metadata. Ordinary application builds omit these watches
and the identity, retaining their normal production behavior.

Active schema-v2 run records use the domain-oriented immutable ID form
`qualification-run-sha256:<64 lowercase hex characters>`, derived from the
unsealed record payload and then sealed with canonical `fingerprintDigest` and
`recordDigest`. Current-state projection selects only valid, compatibility-clean
bundles as current evidence; invalid runs remain historical audit evidence and
cannot replace current qualification state. The production target registry now
contains the Pocket Air Mini target, but the generated matrix classifies it as
unqualified and shows no current valid physical evidence for any declared
workflow. Registration therefore establishes only the immutable target identity;
it does not establish a user-facing support claim. The production-bound harness
is implemented and available for physical runs, but harness implementation by
itself does not qualify a workflow or device.

The operator workflow begins with
`npm --prefix apps/emuchef-app run device-qualification`. An unregistered
device is connected, probed, matched, attested as `usb2` or `usb3`, reviewed,
and explicitly registered; registration requires stopping, committing the
registry and matrix, and rebuilding before workflow qualification. The fresh
build then uses the registered target and canonical workflow, normal EmuChef
inputs, review, explicit real-execution confirmation, and only
workflow-declared human checkpoints. The operator inspects terminal candidate
classification, explicitly records the run when appropriate, commits the
immutable evidence bundle and matrix, and runs `make device-qualification-check`
and repository tests before committing or shipping evidence. Daijisho and ES-DE
remain deferred, and physical matrix work remains in progress until real
evidence is intentionally recorded.

The development-only qualification mode exposes target-registration status and
capture through the existing EmuChef application workflow. Its repository is a
lazy optional provider: ordinary and packaged builds do not resolve, canonicalize,
or expect a trusted source checkout during application startup. When any of the
four runtime gates fail—debug build, compiled `real-execution` feature, embedded
clean-build identity, and `EMUCHEF_DEVICE_QUALIFICATION=1`—the status command
returns an empty disabled projection without initializing the provider or
starting Node. If an opted-in build cannot resolve the trusted checkout or
canonical tool, the command returns a sanitized unavailable projection with no
workflow, target, or candidate data.

An enabled status uses one serialized repository snapshot containing the
canonical `--describe` result, restart-safe candidate summaries, and a
sanitized resumable snapshot for the persisted non-terminal run session, when
one exists. The snapshot includes only opaque handles, workflow intent,
declared checkpoints, and their persisted outcomes/timestamps; it excludes
device handles and filesystem paths. Status hydration does not re-probe a
device or assign new checkpoint timestamps. It keeps
`runtimeContract` separate from the embedded build identity and reports
recordability only when the current trusted Git `HEAD`, tracked worktree
cleanliness, embedded build identity, and in-process lifecycle state all agree.
Candidate previews recompute promotability against those current facts on every
load/list operation while preserving the stored immutable target values and
provenance. A current repository/build mismatch makes the mode non-recordable
until a fresh clean commit/rebuild is observed.

Target capture accepts only an opaque device handle, an opaque authored device
plan, and the attested `usb2` or `usb3` connection enum. Rust resolves the
selected plan through the production probe/match path, obtains identity and
capability facts from the existing production device qualification path, and
obtains root state only through the existing explicit root-check authority.
Captured schema-v2 facts retain `production_observation`,
`explicit_root_check`, or `operator_attestation` provenance. Package-manager
and storage availability map exactly to `apk_install` and
`shared_storage_write`; unavailable or failed root verification rejects the
candidate. Manufacturer, model, and firmware observations must be strings;
only a numeric Android release is normalized to its string representation.
The review preview is read-only and contains the exact typed facts and sources
persisted in the opaque candidate directory.

Target registration and candidate discard are explicit Tauri actions. The
registration action accepts only an opaque candidate handle, delegates
validation and deterministic target identity/mutation to
`tools/device-qualification.mjs`, and returns the canonical target ID plus a
`requiresCommitAndRebuild` consequence. Successful registration also marks the
trusted repository lifecycle dirty; registration, capture, and future
recordable qualification operations then fail closed until a clean commit,
fresh build identity, and clean tracked worktree are observed. Candidate
discard remains an explicit cleanup operation for stale candidates and is
serialized with status and canonical mutations. Repository filesystem work,
Node status, and Node mutations share one operation gate so status cannot read
half-replaced registry/matrix state or race candidate deletion. React receives
only opaque handles and sanitized DTOs; it has no repository, candidate,
evidence, executable, or process authority. No target or physical evidence is
created by the qualification-mode implementation itself.

The ordinary React application includes a development-only qualification
overlay when trusted mode status is enabled. Rust owns qualification lifecycle,
device association and reassociation, validity, and candidate finalization.
Product operations synchronously publish the exact committed typed device
observation and the authoritative review, admission, terminal, or runtime-loss
transition to the active session. A real-execution admission is routed to the
exact qualification attempt that reserved the device when the start began, even
when that attempt had not yet bound the submitted review: an admission for a
review created before the attempt binds it, and an admission whose review was
replaced after reservation fails that attempt closed instead of leaving it
valid. The product-owned real-execution monitor
retains terminal state and its report before qualification observes completion.
Successful Platform-Tools replacement resets root and runtime device authority
under the same transition boundary and invalidates any currently associated
qualification attempt before another checkpoint or finalization can proceed;
qualification persistence failure does not change the successful product
replacement result.
Inventory observations include only currently available devices and retain the
native session epoch, so an unavailable device or a reconnect under a reused
opaque handle invalidates the associated attempt. Explicit root observations
remain authoritative until the session closes, including while terminal
evidence is pending. The final execution gate publishes its exact fresh probe
facts before target validation; after validation, it resolves the selected plan's
profile again from those same facts and the current trusted catalog. Failure to
establish that qualification profile invalidates qualification without changing
the product execution decision or adding a device probe.
Execution reads and exports are projections; they do not observe or advance
product execution lifecycle. Qualification status does not infer or observe new
product transitions. Before projecting status, Rust may recover and reconcile
persisted qualification-session state, including failing closed for an
unproven process handoff, materializing an invalid candidate, or retrying
terminal candidate materialization only when the current build matches the
persisted session identity, retained execution-report and checkpoint evidence
are complete, and the exact authored source again matches the session-start
digests. React remains responsible for presentation and
explicit operator actions only. A genuinely fresh `get_device_qualification`
reconciliation is an authoritative observation and may be published at its
commit seam. Explicit root checks remain outside device observation.

The session snapshot and candidate summaries cross IPC as sanitized presentation
data, including the backend-authored invalidation explanation and a backend-
owned device-selection lock. React renders that state and sends explicit
operator actions for target capture, registration, checkpoint outcomes,
recording, discarding, or abandonment. Workflow changes may trigger a status
refresh, but React does not reconstruct review, admission, finalization,
validity, or process-local handle authority. There are no
`refresh_qualification_session`, `bind_qualification_review`,
`bind_qualification_execution`, or `finalize_qualification_candidate` IPC
commands.

The current persisted session schema contains no process-local device handle or
device association. Legacy session files may include `deviceHandle` for
invalid-recovery audit conversion; that value never restores native device
authority. Rust keeps the live association only in memory and recreates it
after recovery from fresh trusted device observations; React never carries,
infers, or restores the association. RecoveryStore captures the previous
process's clean-handoff proof when it loads the native marker. A restored
session has no device association, so product device selection remains unlocked
until a trusted observation proves the registered target, including its
authored profile identity and all material target facts. Once associated, the
backend reports selection locked; a restored session awaiting reassociation or
a closed session reports it unlocked. No UI query establishes reassociation.
RecoveryStore captures the prior-process clean-handoff proof at native load and
tracks which qualification candidate began in the current process separately
from resettable frontend session state. This prevents a frontend reset from
reclassifying a same-process attempt using the stale startup marker. Recovery
checks prior-process handoff proof before comparing build identity. An
otherwise-valid persisted attempt with an unproven handoff is durably
invalidated and retained as invalid/not_observed under the current build. After
a clean handoff, an otherwise-valid attempt captured by another build remains
deferred and unchanged, while reserving the one-attempt qualification harness
globally. It has no process-local device, review, or execution association in
the other build, and new sessions, target registration, other inactive-only
mutations, and real execution admission remain blocked until the deferred
candidate is discarded or the captured build resumes it. If
multiple otherwise-valid open attempts are found, recovery selects none and
keeps the harness reserved until the ambiguous candidates are resolved. Under
a matching build, a session resumes when execution was never admitted or when
an admitted execution has a retained terminal observation. If execution was
admitted but no terminal observation was retained, recovery records a
lifecycle-transition-missed failure as invalid/not_observed audit evidence; a
matching build cannot repair that gap.
An incompatible or already-invalid persisted attempt may be closed under
another build only as invalid/not_observed evidence that cannot be promoted.

Qualification sessions use opaque handles and strict Rust-owned persistence
beside the canonical candidate envelope. Source digests are captured when the
trusted session starts and revalidated against the clean canonical checkout
before valid candidate materialization. If source authority changes, the
completed session remains safely pending until the original fingerprint can be
verified again. Terminal observation time is captured when the product
transition arrives. Automatic finalization uses those retained values and the
exact sanitized execution report; a dirty later checkout cannot rewrite their
provenance. Required checkpoints recorded after terminal
execution can complete an awaiting candidate. Invalid, abandoned, corrupt, or
unproven recovery states remain fail-closed and discardable; persistence or
finalization failures are durably poisoned so a clean restart cannot resume
them as valid. Persisted qualification-run candidates remain visible
independently of an active session and require an explicit record or discard
action. Canonical run IDs, fingerprints, record digests, and evidence mutation
remain owned by `tools/device-qualification.mjs`.

## Phase 6D.6 physical interruption qualification

The ignored Rust physical harness is fail-closed and fail-reporting: explicit
execution returns a test error for missing gates, blocked checkpoints,
unexpected terminal issue codes, incomplete step accounting, unsafe cleanup, or
an incomplete scenario contract. It runs one exact scenario and repetition
through a reviewed execution plan and the real ADB executor boundary.

The checked-in scenario manifest is the authority shared by the Rust harness
and the dependency-free Node evidence validator. Each contract specifies
expected execution type and issue codes, accepted step/Not-attempted counts,
partial-change and authority dispositions, exact target-process facts,
production-slot lifecycle, host deadline-clock measurement and phase,
transition chronology, UI artifacts, cleanup, and residual requirements.
Evidence includes canonical record and trace digests plus unique run, scope,
sentinel, nonce, path, and slot identities; copied or relabelled records are
rejected.

Active-cancellation qualification requires the exact target mutation child to
be spawned, started, and alive immediately before cancellation, with the action
strictly before the child's terminal event and bound to the same run and
operation. A runner callback, post-operation probe, delayed poll, or local
boolean is insufficient. The harness now binds a production-owned process
observation to the first reviewed host `Push` and creates `active-ready` only
after sampling that exact child alive. A safe-boundary cancellation uses a
distinct finished-before-request phase. The active-slot record observes the
RAII guard owning the production execution-session slot, carries the exact
run-scope and execution identity, and cannot pass on an auxiliary lease,
shadow flag, early release, or another run's lifecycle.

Same-serial replacement qualification polls successful ADB inventory samples
and stable fingerprints to prove original attachment, a serial-absent interval,
replacement attachment after disconnect, changed identity, and no simultaneous
target. Authorization qualification is a safe-boundary reconnect case: the
first operation completes, stored trust is revoked, the exact selected serial
is proven absent, and the same serial must reconnect as genuinely
`unauthorized` before `operator-action` releases the second operation. The
second operation must fail before mutation. Its terminal issue may be
`device_unauthorized`, or `device_identity_unverified` when the production
pre-operation identity guard cannot collect complete evidence from that
independently observed unauthorized device. The identity branch never qualifies
without the exact authorization chronology. Explicit reauthorization and a
final authorized cleanup state remain required; changed identity, generic
identity failure, offline, disconnect, and mismatched issue evidence are
insufficient. Earlier blocked attempts remain non-passing audit evidence under
their exact historical contracts.
Physical `device_offline` evidence is conditional diagnostic evidence rather
than a closure requirement. ADB offline is normally a transient transport
initialization or failure state with no general operator-controlled transition.
The harness and schema continue to accept truthful offline attempts, while the
twelve-scenario mandatory matrix uses the active and boundary USB-disconnect
cases as its physical transport proof.
Development-build UI smoke is mandatory closure evidence: two composite
records each contain cancellation, transport, root, storage, and
host-sleep/runtime-loss subcases. The transport subcase binds only to passing
active or boundary USB-disconnect evidence, not conditional offline evidence.
Every subcase binds an exact physical backend
run and trace to a distinct UI sub-run, development-build digest, exact authored
projection, **Not attempted** count, partial-change and authority recovery
state, forbidden-control absence, canonical UI-state artifact, and
artifact-bound operator observation. Nested unsafe text is rejected.

The mandatory `operation_timeout` physical scenario uses the reviewed first
copy step with the exact hard-coded private pseudo-device paths `/dev/zero`
(source) and `/dev/null` (destination). No FIFO or other special file is
created on the device; the real path is `copy_files` with
`source.location == "device"` through `RealAdbDevice::copy_on_device` and the
owned `DeviceCopy` process. A private, thread-local, one-shot `#[cfg(test)]`
deadline override selects exactly 15 seconds for this ignored qualification
entry point; production `ProcessOperation::DeviceCopy.deadline()` remains 300
seconds and has no public or environment-controlled timeout configuration. The
exact child must be sampled alive about 12 seconds after its matching
mutation, then the actual timer transition must be recorded as
`DeadlineReached` before existing kill/reap cleanup and `Terminal`. Passing
evidence uses one opaque operation identity, `actionKind: "deadline_reached"`,
confirmed process cleanup, `operation_timed_out`, a failed first step with the
second step Not attempted, and clean run-scope residual verification; the
unique run scope remains authoritative even though the timeout copy creates no
persistent payload. No operator marker or Terminal 2 procedure is used for
this scenario; active host-push observation remains limited to cancellation,
USB-disconnect, and conditional offline cases. The `/dev/zero` and `/dev/null`
paths are private to this qualification and never enter evidence, cleanup
inventory, authored plans, or ordinary execution.
The timeout lifecycle observer may contain unrelated Probe, Predicate, or other
operation events. Timeout evidence selects exactly one `DeviceCopy`
`DeadlineReached` event at 15 seconds, then requires that selected operation's
single Spawned, MutationStarted, live LivenessSampled, DeadlineReached, and
Terminal events in raw chronological order. Missing, duplicate, contradictory,
wrong-class, wrong-deadline, or mixed-identity target events are rejected while
unrelated operation IDs are ignored.

Host-sleep qualification uses the same private `/dev/zero -> /dev/null`
stimulus with a 120-second scoped qualification deadline and an exact
deadline-clock observation seam. The operator creates `sleep-requested` while
awake; the harness proves the exact child alive, samples the exact deadline
clock, and creates the internal `sleep-ready` marker; the operator then creates
`sleep-entered` within four seconds (the final awake handoff and activeProcess
action boundary), suspends the host, and creates `wake` immediately after
resume. The post-wake sample is requested only after `wake` is observed and may
legitimately follow the owner terminal sample when the deadline became ready
during suspension. The measurement tolerance is 8,000 ms and the timer
classification derives from measured clock advancement, wall duration, and
remaining budget; `indeterminate` and `contradictory` block. The owner-recorded
`DeadlineReached` event is authoritative for whether the deadline branch won:
a timeout requires it, and completion at the deadline boundary without it is
not relabelled as a timeout. `sleepEnteredAt` means the last operator/harness
handoff immediately before physical suspension, and `wakeAt` means the first
post-resume acknowledgement; neither claims an exact OS event timestamp.
Physical host-sleep repetitions remain unqualified.

Schema-v1 compatibility is additive for historical non-timeout records: their
`timeout` object and `activeProcess.actionKind` may be absent. Every
`operation_timeout` record must carry exactly the four timeout fields with the
300,000 ms production deadline, 15,000 ms scoped qualification deadline, and
`test_only_scoped_override` source. `processCleanup` is one of `confirmed`,
`uncertain`, or `not_observed`; only a passing timeout record requires
`confirmed`. A non-null timeout `activeProcess` always requires
`actionKind: "deadline_reached"`, while a passing timeout still requires that
process evidence to be present.

Low-storage qualification requires a disposable selected device with between
4 GiB and 5,308,416 KiB free before mutation, a verified fixture-owned 1 GiB
recovery reserve, a bounded run-scoped filler capped at 4 GiB with 64 MiB of
cleanup headroom, a generated 128 MiB host fixture payload for genuine
production-path ENOSPC, and verified payload/filler/sentinel/reserve cleanup
with free-space restoration.
The evidence validator retains failed, skipped, and blocked attempts with
their truthful cleanup/residual facts, but only passing records can satisfy a
scenario contract or count toward matrix completion. The host-only delay seam
is a one-shot, thread-local regression arm for exactly one DeviceCopy process;
it cannot delay status/output polling, turn an exited child into a timeout, or
qualify physical evidence. Identity probes and other operations are unaffected,
and the arm clears on normal, timeout, panic, or parallel return.

The development UI-smoke binding/capture plumbing is implemented; manual
UI-smoke qualification remains deferred until the required compatible
host-sleep physical binding exists and the operator chooses to perform the
deferred manual work; the final five-subcase composite additionally cannot be
completed until a compatible passing host-sleep physical binding exists;
`identity_replacement` and the remaining host-sleep physical repetitions are
still required for Phase 6D closure; both UI-smoke repetitions themselves
remain missing because manual UI-smoke qualification has not been run.
`tools/phase-6d6-evidence.mjs` derives and verifies the
checked-in `docs/testing/phase-6d6/ui-binding-index.json`, which lists only
UI-contract-compatible passing physical bindings plus source and raw
evidence/trace digests; default validation is read-only and
`--regenerate-ui-binding-index` writes the index only after the base evidence
contract passes. A debug-only Tauri bridge (`phase6d6_ui_smoke.rs`) activates
only with the `real-execution` Cargo feature,
`EMUCHEF_RUN_REAL_ADB_TESTS=1`, and `EMUCHEF_PHASE_6D6_UI_SMOKE=1`; it verifies
the index self digest, source digests, exact evidence/trace raw bytes, and the
parsed run/record/trace identities itself, then projects a fixed terminal
report through the production real-execution projection and renders it in the
normal React terminal UI. React receives and sends only opaque handles plus the
UI repetition. Capture writes a canonical create-new `ui_state_capture`
artifact under `docs/testing/phase-6d6/evidence/ui/` bound to the exact backend
run/trace and trusted development-build identity. The application never
creates the operator observation or the final `ui_smoke_composite` record.
Accepted passing physical evidence exists for `cancellation_active`,
`cancellation_boundary`, `usb_disconnect_active`, `usb_disconnect_boundary`,
`device_unauthorized`, `identity_stability`, `root_revocation`, `low_storage`,
and `operation_timeout`. Only the two accepted `usb_disconnect_active` records
with `device_transport_lost` satisfy the mandatory transport UI contract;
passing `usb_disconnect_boundary` records reporting `device_disconnected`
remain accepted physical evidence but are excluded from the UI binding.
`identity_replacement` repetitions 1–2, `host_sleep_before_deadline`
repetitions 1–2, `host_sleep_after_deadline` repetitions 1–2, and
`ui_smoke_composite` repetitions 1–2 remain missing; no UI-smoke repetition has
been run or counted. `device_offline` remains conditional diagnostic evidence.

Phase 6D remains In Progress until all twelve mandatory scenarios have two clean,
sanitized, contract-valid passing repetitions, both UI-smoke repetitions pass,
and the complete automated matrix is green. `device_offline` remains supported
conditional evidence and does not contribute to the missing-repetition count.
The exact backend `clippy -D warnings` gate passes; the Tauri strict Clippy
gate now passes under both the default and `real-execution` feature sets,
resolving the lint findings that previously reproduced identically in an
isolated clean checkout at `HEAD` (`b8bf14a`), so repository-wide strict Clippy
is green. The validator is the authority for accepted and missing repetitions,
and blocked mandatory scenarios do not count as closure.

## Recipe qualification current state

Standalone RetroArch automated qualification covers the real
`app.retroarch.provision` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/retroarch/qualification-contract.json`,
and the active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs`.
The qualification uses the real authored catalog through
`runtime_configuration::plan_configuration` with the
`ayaneo.konkr_pocket_fit.base` device plan, exercises the production review
projection, and executes the unchanged generated plan through
`ExecutorAdapters::with_sandbox_roots`. It qualifies the App Definition launcher
alias `com.retroarch.browser.mainmenu.MainMenuActivity` and the plan-local
package/activity operands of all three launches and two force stops. Both the
RetroArch Recipe and App Definition source bytes are digest-bound. It also
qualifies the authored first-launch/bootstrap lifecycle as bootstrap launch ->
1500 ms wait -> force-stop followed by permission launch -> 5000 ms wait ->
force-stop, including generated
plan dependency/order checks, successful deterministic execution records, and a
test-private lifecycle failure regression that preserves prior results and blocks
dependent work. It also preserves optional configuration behavior, repeated-install
skip behavior, PPSSPP verification failure semantics, and production review
coverage without live public network access or ADB. Physical and full end-to-end
qualification remain deferred.

Standalone BIOS automated qualification covers the real authored
`feature.copy_bios` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/bios/qualification-contract.json`, bound
to the raw authored recipe SHA-256
`4f4471aacb0cacebffcebb4785b12f0e7ef43d306526031540d0496957840fbc`, and its
active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`.
Qualification uses `ayaneo.generic.base` as production capability context but
explicitly selects only `feature.copy_bios`. It proves production planning and
review, required-input rejection, recursive nested copy through normal sandbox
adapters, and authored destination-verification failure through a private test
device wrapper. The current BIOS Recipe uses RetroArch's named `system`
target, resolved to `/storage/emulated/0/RetroArch/system`; physical execution
remains deferred.

Standalone ROM/content-copy automated qualification covers the real authored
`feature.copy_roms` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/roms/qualification-contract.json`, bound
to the raw authored recipe SHA-256
`956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041`, and its
qualification document is `docs/product/recipe-qualification-roms.md`.
Qualification uses `ayaneo.generic.base` as production capability context but
explicitly selects only `feature.copy_roms`. It proves production planning and
review, required/default/alternate/invalid bindings, deterministic nested
`merge`, `replace`, and directory-style `sync` execution, and truthful copy
failure reporting without ADB, live network, physical hardware, or packaged
GUI work. The authored `verify: []` remains without verification-predicate
coverage. Directory-style `sync` is the authored “Mirror source” contract and
removes destination-only files. Legacy path-list `sync` has no single reviewed
source boundary and fails closed as unsupported before mutation; it is not
timestamp-based sync and is not mapped to `Add/Update`. Physical qualification is
deferred with cleanup authority
`not_authorized_for_recipe_qualification`.

Standalone Obtainium automated qualification covers the real authored
`app.obtainium.install` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/obtainium/qualification-contract.json`,
bound to the raw authored recipe SHA-256
`378cc9b519a9872a9a70667331c1a6a210aee3958d7d61abf66b55c0ecc4395d`, and its
active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_obtainium_tests.rs`.
Qualification uses `ayaneo.generic.base` only as the production planning and
capability context and explicitly selects only `app.obtainium.install`; the
device plan does not contain Obtainium and is not treated as product
provenance. The qualification covers production planning and review, authored
GitHub stable latest-release policy and asset-pattern preservation, including
plan serialization and parsing. Deterministic release metadata selects the
newest eligible stable release while an older release remains cached; the
selected asset's URL determines the cache entry. The resulting plan resolves
and installs the App Definition-owned APK without live GitHub access or ADB.
Qualification also covers release and asset provenance, package-state-driven
repeated-install skipping, package-identity rejection before device mutation,
and truthful install-failure semantics through a private test-only device
adapter. The standalone direct-URL source strategy remains independently
validated and tested. Physical qualification is not claimed.

Physical qualification for all three standalone workflows is deferred by owner
with cleanup authority `not_authorized_for_recipe_qualification`.
The default `ayaneo.konkr_pocket_fit.base` selection includes both
`app.retroarch.provision` and `feature.copy_bios`, but current planning rejects
that composition with `app_target_destination_overlap`. Both recipes copy to
the same `system` App Target: RetroArch uses additive `merge`, while BIOS uses
directory `sync`, which removes destination-only files. CopyCommit reviews
capture each destination manifest before mutation and do not compose sequential
operations on one boundary, so the combined workflow is unsupported. The
authored recipes and production destinations remain unchanged. The earlier
sandbox characterization does not establish this combined flow as supported.
The fake filesystem does not treat `/sdcard` and `/storage/emulated/0` as
aliases. Phase 6E remains in progress for other workflow qualification and
physical/full end-to-end work; Phase 6D retains its missing physical and
UI-smoke evidence. This work does not waive those requirements.

### Protected App Target qualification

The generic protected App Target qualification is reproducible from source
using the disposable application package `com.emuchef.fixture`; it covers
runtime-generated file trees. The optional RetroArch package/build/APK
qualification is reserved for T15. The production RetroArch App Definition and
ROM Recipe destination `/sdcard/ROMs` remain authoritative. App-private
directory copies from host sources capture one typed snapshot before review.
App Target sources that reference runtime step outputs are rejected during
review preparation because those outputs cannot be included in a stable source
manifest before execution.
The execution plan binds the operation, source path and manifest digest,
selected destination identity, destination-before manifest digest, and exact
deletion set. Review actions disclose every entry in that deletion set.
Immediately before the first mutation, execution revalidates the complete source
and destination trees against the reviewed snapshots. Drift found at that point
rejects the operation before mutation begins. Each destination entry's identity
and file digest are checked immediately before its overwrite or deletion. Each
source file is opened without following symlinks and its digest is verified
immediately before publication, so changed source content is not published.
Drift detected after mutation begins rejects the affected publication and is
reported as a truthful partial outcome that may include reviewed deletions or
earlier publications already completed. Incomplete observation may still make
the result uncertain.

Mirror compares both relative path and entry kind. A destination file at a path
that the source uses as a directory is included in the reviewed deletion set
and removed before publication, so the resulting tree matches the source tree.
Fake copy-commit reports with `succeeded: false` fail the step and execution,
including reports that describe partial mutations. After final destination
verification succeeds, helper or staging cleanup errors remain pending cleanup
and do not change the setup result. A successful copy with staging cleanup
pending retains its successful step and deployment status and adds an
owner-visible warning with no local staging path. Host source files are opened
relative to a retained source-root descriptor without following symlinks,
checked as regular files, and verified against each reviewed SHA-256 while
copied. Android checks the staged source digest and the exact publication
temporary file before rename. Copy-commit source access requires Unix
descriptor-relative no-follow filesystem operations; hosts without those
operations reject the copy instead of using path-based opens. Non-Unix host
support for this copy path is not qualified.
The planner rejects selected CopyCommit operations whose destination paths are
equal or ancestor/descendant, including overlaps across recipes. Disjoint
sibling boundaries remain eligible. The review protocol has no sequential
same-boundary composition: each snapshot is captured before mutation, so one
operation cannot safely consume another operation's output. For Android shared
storage, overlap checks also recognize `/sdcard` and `/storage/self/primary`
as aliases of `/storage/emulated/0`; unresolved destination identity fails
closed.

Shared-storage CopyCommit runs as Android shell UID 2000 and never falls back
to root. For `/sdcard` destinations, it preserves the reviewed spelling and
accepts only the verified alias chain `/sdcard` to `/storage/self/primary`
to `/storage/emulated/0`. It checks the selected-tree identity before review
and again immediately before mutation. Unsupported permissions, aliases, or
identity changes fail closed and leave the result uncertain when mutation
state cannot be established. Shared-storage Mirror removes only the exact
reviewed deletion set and verifies staged payloads before applying changes.
The manual shared-storage qualification uses an isolated directory below
`/sdcard/ROMs` and a sibling sentinel; it does not modify the production ROM
Recipe destination or RetroArch definition.


## Prepared execution reviews

The native application plans the selected configuration, then sends that plan,
the selected target device, and `real` mode to the trusted
`prepareExecutionReview` sidecar operation. It retains the exact prepared plan,
canonical digest, CopyCommit source and destination snapshots, and
backend-authored review under one review handle. The UI presents the prepared
review projection, including the selected boundary and complete copy additions,
replacements, and deletions. Real Apply sends the exact retained plan and
digest to `startExecution`; the backend revalidates CopyCommit identities and
manifests immediately before mutation. Simulated execution consumes the plan
and digest from its retained prepared response. The fake copy-commit route
derives its isolated destination baseline from the reviewed destination
manifest's paths and entry kinds, then verifies that the simulation deletion
set matches the reviewed deletion set. Empty placeholder files model reviewed
file paths; the simulation does not copy real-device file bytes into its fake
tree. The fake destination is seeded only from the retained review snapshot and
must match that path and kind baseline before mutation. The backend advertises
`prepareExecutionReview` as a required protocol capability.
