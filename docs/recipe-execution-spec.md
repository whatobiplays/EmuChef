# EmuChef Artifact-Centric Execution Spec

## 1. Purpose

EmuChef provisions Android emulation devices from declarative recipes. The system is designed to:

- install and launch apps
- grant permissions and appops
- download remote artifacts such as APKs and ZIPs
- extract archives on host or device
- copy files into shared storage or app-private paths
- support reusable batching patterns such as grouped RetroArch core installation

The current design goal is to keep authored recipes declarative and predictable, while keeping execution semantics in the executor rather than the planner.

## 2. Status

### Implemented baseline

The following is now the intended baseline design and has been implemented at a structural level:

- map-based recipe schema for `inputs`, `artifacts`, and `artifact_groups`
- flattened literal params with `{ ref: ... }` only for references
- normalized planner output with explicit ref objects instead of planner-resolved concrete values
- centralized step type registry/spec model
- runtime ref resolver in the executor
- artifact-driven steps:
  - `resolve_artifacts`
  - `extract_artifacts`
  - `extract_archive`
  - `copy_files`
- declarative permission plan without embedded ADB command tuples
- single-threaded executor
- grouped artifact support for flows like RetroArch core seeding

### Approved next hardening

The following is approved next work, based on the first real device run, but should be treated as executor hardening rather than already completed unless separately verified:

- distinct `BLOCKED` status for downstream steps after upstream failure
- better `launch_app` behavior using explicit activity or resolved launcher component before `monkey`
- typed artifact download failures, including clearer TLS trust-store errors

## 3. Design Principles

### 3.1 Planner is declarative

The planner decides:

- what should happen
- what defaults apply
- what refs mean
- what artifacts/groups expand to
- what permission intent exists

The planner does not decide:

- how to invoke ADB
- how to download a file
- how to extract archives
- how to copy files
- how to generate shell commands

### 3.2 Executor owns execution

The executor owns:

- downloads and caching
- ZIP extraction
- ADB operations
- copy mechanics
- runtime permission command generation
- runtime ref resolution
- verify checks

### 3.3 Authored YAML should stay simple

Recipe authors should mostly write literals directly and only use `{ ref: ... }` where a runtime-produced value is needed.

### 3.4 Runtime state is explicit

Step outputs, artifact resolution, and input bindings are all represented as runtime state, not implicit side effects.

## 4. High-Level Architecture

The system has three layers.

### 4.1 RecipeDefinition

This is the authored YAML parsed into domain objects.

It preserves author intent and schema structure.

### 4.2 ExecutionPlan

This is the planner output.

It is:

- validated
- normalized
- explicit
- flattened

The executor consumes only this.

### 4.3 ExecutionState

This is mutable runtime state during execution.

It tracks:

- artifact runtime state
- step runtime state
- resolved inputs
- step statuses
- outputs
- errors

## 5. Authored Recipe Schema

### 5.1 Top-level shape

A recipe uses:

- `schema_version`
- `kind`
- `id`
- `name`
- optional `description`
- optional `provides`
- `inputs`
- `artifacts`
- `artifact_groups`
- `permissions`
- `steps`

### 5.2 Inputs

`inputs` is a map keyed by input ID.

Example:

```yaml
inputs:
  retroarch_cfg:
    type: file
    required: false
    multiple: false
    default: null
```

This replaced the older list-based input model.

### 5.3 Artifacts

`artifacts` is a map keyed by artifact ID. Schema-v1 Recipes declare either a
Recipe-owned `remote_file` or a reference to an App Definition-owned
`app_artifact`.

`remote_file` requires a literal `url` and does not accept `input.ref`. A
Recipe input used by an App Artifact is bound through the `user_provided`
source strategy. Both declaration types may set the Recipe-owned `cache`
field to `default` or `none`; it defaults to `default`.

`app_artifact` requires `app_ref` and `artifact`. Its source strategy is
defined by that named App Definition artifact. A `user_provided` artifact
requires an `input` mapping with `ref: inputs.<id>`; the reference must identify
one `file` input with `multiple: false` declared by the same Recipe. Other
source strategies do not accept `input`; `direct_url` and `latest_release`
artifacts use their App Definition source policy.

A `user_provided` App Artifact is represented in the execution plan as a
`local_input` source. Its cache setting is validated and retained as Recipe
policy, but the supplied bytes are copied to a guarded execution-scoped
snapshot and are never persisted to or reused from the shared artifact cache.

Examples:

```yaml
artifacts:
  retroarch_apk:
    type: remote_file
    url: https://buildbot.libretro.com/nightly/android/RetroArch_aarch64.apk
    cache: default
  user_supplied_asset:
    type: app_artifact
    app_ref: example_app
    artifact: user_asset
    input:
      ref: inputs.user_asset_file
    cache: default
```

### 5.4 Artifact groups

`artifact_groups` is a map keyed by group ID.

Each group contains a list of artifact IDs.

Example:

```yaml
artifact_groups:
  retroarch_cores:
    - core_gambatte_zip
    - core_snes9x_zip
```

Purpose:

- batch related artifacts without adding one step per artifact

### 5.5 Permissions

Legacy package-based permission declarations remain declarative in the
`grant_permissions` step parameters until their compatibility cutover. New
App Definition permission steps select one app-owned set directly on the
step:

```yaml
- id: grant_retroarch_baseline
  type: grant_permissions
  app_ref: retroarch
  params:
    permission_set: baseline
    policy:
      on_failure: warn
```

The app-aware form does not author package names or duplicate runtime/app-op
action arrays. The selected App Definition supplies those actions.

Supported categories include:

- `runtime`
- `appops`
- existing `manual` compatibility path if already present in authored schema
- `policy`

Policy fields include:

- `on_failure`
- `require_all`

### 5.6 Steps

`steps` remains an ordered list.

Each step has:

- `id`
- `type`
- optional `name`
- `dependencies`
- `constraints`
- `skip_if`
- `params`
- `verify`
- `user_toggleable`

## 6. Authoring Rules for Params and Refs

### 6.1 Literals are written directly

Authors should write:

```yaml
dest: /sdcard/RetroArch/assets
package_name: com.retroarch.aarch64
duration_ms: 1000
replace_existing: false
```

### 6.2 Refs use `{ ref: ... }`

Example:

```yaml
source:
  ref: steps.extract_selected_cores
```

### 6.3 Supported authored ref forms

Recipe authors use recipe-local refs:

- `inputs.<id>`
- `artifacts.<id>.<field>`
- `steps.<id>`
- `steps.<id>.outputs.<field>`

### 6.4 Shorthand step refs

A bare step ref:

```yaml
ref: steps.extract_selected_cores
```

is valid only if that step type defines a primary output.

The planner rewrites it to:

```yaml
steps.extract_selected_cores.outputs.extracted_paths
```

This shorthand must not survive into executor-facing plan data.

## 7. Internal Param Model

Planner-normalized params use two forms only:

- `LiteralParam(value=...)`
- `RefParam(ref=...)`

Parsing rule:

- if a value is a mapping with exactly `ref`, treat it as a `RefParam`
- otherwise treat it as a literal

This keeps authoring simple while preserving strong internal normalization.

## 8. Ref Model

### 8.1 Executor-time explicit refs

The executor supports only explicit refs:

- `inputs.<id>`
- `artifacts.<id>.<field>`
- `steps.<id>.outputs.<field>`

If execution-plan-global namespacing is used internally, that is planner-internal only and must not leak into authored YAML.

### 8.2 Resolver ownership

Refs are resolved by one central runtime resolver.

Individual step handlers must not parse refs on their own.

### 8.3 Resolver errors

Structured runtime ref errors include:

- `invalid_ref_format`
- `unknown_input_ref`
- `unknown_artifact_ref`
- `unknown_artifact_field`
- `artifact_not_resolved`
- `unknown_step_ref`
- `unknown_step_output`
- `step_output_unavailable`

### 8.4 Step output availability

In v1:

- step outputs are readable only from succeeded steps
- skipped steps produce no outputs
- failed steps produce no outputs

## 9. Runtime Value Model

Runtime values are typed.

Supported kinds:

- `file_path`
- `directory_path`
- `path_list`
- `string`
- `integer`
- `boolean`
- `object`
- `null`

This allows steps like `copy_files` to accept:

- one file
- one directory
- a list of paths

without guessing from raw strings.

## 10. Runtime State Model

### 10.1 ArtifactRuntimeState

Tracks per-artifact runtime status, including:

- artifact ID
- resolution status
- local path
- resolved URL
- filename
- cache hit
- error

### 10.2 StepRuntimeState

Tracks per-step runtime state, including:

- step ID
- status
- outputs
- error

### 10.3 ExecutionState

Tracks:

- plan ID
- overall execution status
- resolved inputs
- artifact runtime states
- step runtime states
- events/logs

## 11. Step Type Registry

A central step-spec registry defines step behavior.

Each step spec includes:

- `type_name`
- `primary_output_name` or `None`
- param specs
- defaults
- allowed enum values
- optional planner normalization hook
- executor dispatch target/handler

This keeps planner and executor logic from fragmenting into step-specific conditionals everywhere.

## 12. Supported Step Types

### New artifact-centric steps

- `resolve_artifacts`
- `extract_artifacts`
- `extract_archive`
- `copy_files`

### Existing retained steps

- `install_apk`
- `grant_permissions`
- `launch_app`
- `force_stop_app`
- `wait`

### Legacy steps

Older copy/push steps may still exist for compatibility, but they are not the forward path and should not be extended.

## 13. Step Specifications

### 13.1 `resolve_artifacts`

Purpose:

- resolve one or more declared artifacts into local runtime materialization

Params:

- `artifacts`: optional list
- `artifact_groups`: optional list

Rules:

- at least one of `artifacts` or `artifact_groups` must be present
- planner expands groups into a flat artifact list
- duplicates after expansion fail validation
- no primary output shorthand

Outputs:

- none intended for authoring
- main effect is mutation of artifact runtime state

### 13.2 `extract_artifacts`

Purpose:

- extract one or more resolved archives

Params:

- `artifacts`: optional list
- `artifact_groups`: optional list
- `extract_on`: optional, default `host`
  - `host`
  - `device`

Rules:

- planner expands groups into a flat artifact list
- duplicates after expansion fail validation

Primary output:

- `extracted_paths`

### 13.3 `extract_archive`

Purpose:

- extract one archive from a ref

Params:

- `archive`: required ref
- `extract_on`: optional, default `host`
- `dest`: required only when `extract_on=device`
- `device_temp_path`: optional
- `cleanup`: optional, default `true`

Rules:

- reject irrelevant params
- device-specific params are only valid in device extraction mode

Primary output:

- `extracted_path`

### 13.4 `copy_files`

Purpose:

- unified replacement for file/tree copy and BYO copy flows

Params:

- `source`: required ref
- `dest`: required literal device path, compatible input ref, or single-key
  `{ app_target: <target-id> }` value when the step has matching `app_ref`
- `copy_policy`: optional, default `merge`
  - `merge`
  - `replace`
  - `sync`

Source may resolve to:

- `file_path`
- `directory_path`
- `path_list`

Named targets resolve during Rust planning to Android user-0 device paths.
Directory and path-list sources require a directory target. A single file may
use a directory or exact file target; `sync` requires a directory target. A
target at `app_data` or `external_app_data` adds `app_data_write` to the step's
effective capabilities, and `shared_storage` adds `shared_storage_write`.
Conditions using a named target inherit the step's `app_ref`; the reviewed plan
contains only concrete paths and effective constraints.

Primary output:

- `copied_paths`

V1 destination semantics:

- if source is `directory_path` or `path_list`, `dest` is treated as a destination directory
- if source is `file_path` and `dest` exists as a directory, copy into that directory
- otherwise treat `dest` as the exact target path
- no trailing-slash inference or heuristic guessing

V1 scope:

- `dest` is device-only

### 13.5 `install_apk`

Params:

- `app`: required ref
- `replace_existing`: optional, default `false`

Runtime validation:

- resolved file must end in `.apk`

### 13.6 `launch_app`

Params:

- `package_name`: required literal
- `activity`: optional literal

Behavior:

- if `activity` is present, launch explicit component
- otherwise use default launch path
- improved launcher resolution before `monkey` is approved next hardening

### 13.7 `force_stop_app`

Params:

- `package_name`: required literal

### 13.8 `wait`

Params:

- `duration_ms`: required positive integer

### 13.9 `grant_permissions`

Legacy package-based steps retain their existing `runtime`, `appops`, and
legacy policy parameters. App-aware steps require `app_ref`, a literal
`permission_set` of `baseline` or `elevated`, and `policy.on_failure` of
`warn` or `fail`.

The app-aware permission step is planned into the step-local typed
`ExecutionStep.permission_plan`. This plan is the sole executable source for
new-form permission actions. `ExecutionStep.params` contains no duplicate
action arrays. The plan retains the App Definition package, action identity,
API bounds, resolved applicability, permission set, and failure policy. The
canonical plan digest covers the complete step-local plan. Review, execution,
and reporting consume the same retained plan and app snapshot.

Behavior:

- planning resolves every selected App Definition action before capability
  pruning and validates the action package against the selected app snapshot
- bounded actions require a known Android API level; unknown API applicability
  is a planning error with the action identity and bounds, while unbounded
  actions remain applicable with an unknown API level
- when a selected target supplies an API level, planning uses it ahead of
  profile context and retains it in the plan's device context for review
- a known API outside an inclusive action bound retains a `not_applicable`
  result with its confirmed reason
- `baseline` requires `shell_command` and uses ordinary ADB shell execution
- `elevated` requires both `shell_command` and `root_shell` and is omitted
  when either capability is unavailable
- elevated execution requires reviewed root authorization and a fresh
  per-command root revalidation; the actual operation uses only typed runtime
  grants and app-op `allow` updates
- a retained step with no applicable actions is skipped before shell execution
  or root preflight and preserves every action's applicability reason;
  capability-pruned steps do not appear in the plan
- ordinary action failures do not stop other independent applicable actions;
  `warn` records failed actions as warnings and yields
  `succeeded_with_warnings`, while `fail` fails the step and adds a report error
- root-authority, identity, transport, and interruption failures remain
  immediate safety fail-stops
- legacy package-based steps preserve their existing behavior until the
  compatibility cutover

## 14. Planner Responsibilities

The planner is responsible for:

- schema validation
- dependency validation
- ref syntax validation
- static target validation where knowable
- step shorthand ref rewriting
- artifact group expansion
- duplicate detection
- default injection
- param normalization into `LiteralParam` / `RefParam`
- permission plan emission as structured intent only

The planner is not responsible for:

- resolving runtime step outputs
- downloading artifacts
- generating ADB commands
- generating shell commands
- archive extraction mechanics
- copy mechanics

### 14.1 Static target validation

For step-output refs, planning validates only:

- the referenced step exists
- the referenced output name is valid for that step type

Planning does not require:

- that the producing step be an explicit dependency
- that the runtime output already exist

If the output is unavailable at execution time, runtime fails.

### 14.2 Default injection

Current defaults:

- `copy_files.copy_policy = merge`
- `extract_artifacts.extract_on = host`
- `extract_archive.extract_on = host`
- `extract_archive.cleanup = true`
- `install_apk.replace_existing = false`

## 15. Permission Plan Model

### 15.1 App Definition execution plan

For a new-form permission step, the planner emits one typed permission plan on
that `ExecutionStep`. No top-level permission plan or alternate executable
action collection exists. The retained app snapshot binds the action package to
the selected App Definition. The plan digest covers the action sequence and
its resolved facts.

The plan does not contain arbitrary command strings. Each closed action maps to
one fixed adapter operation:

- runtime permission grant through `pm grant`
- app-op update through `appops set <package> <operation> allow`

The action sequence carries:

- selected permission set and `warn`/`fail` policy
- action kind and app snapshot package identity
- runtime permission or app-op identity
- authored API bounds and planner-resolved applicability
- a confirmed reason when the target API is outside those bounds

### 15.2 Applicability and execution outcomes

- bounded actions with an unknown target API block planning with actionable
  diagnostics; actions without bounds do not require the API level
- an action outside a known inclusive API range remains in the plan with a
  `not_applicable` reason
- a selected step with no applicable actions is skipped with action reasons
  preserved and no device shell/root preflight; a capability-pruned step is
  absent from the selected plan
- each attempted action receives an `executed` or `failed` result. Ordinary
  failures continue through independent actions before applying the step
  policy. Immediate safety failures stop at once.

### 15.3 Capability and root authority

- `baseline` requires `shell_command` and executes without `su`.
- `elevated` requires both `shell_command` and `root_shell` and is pruned when
  either capability is unavailable.
- The shared Rust/Tauri root-work contract classifies a new-form elevated
  permission step as root-dependent. Legacy permission steps remain outside
  this new root classification.
- Capability eligibility, reviewed root authorization, root preflight,
  per-command root revalidation, and actual command privilege are separate
  checks. An elevated command uses the existing guarded `su -c` adapter only
  after the plan grants reviewed root authority and the fresh root probe passes.

RetroArch's `MANAGE_EXTERNAL_STORAGE` action belongs to `elevated`, so it runs
only on rooted devices. Its baseline runtime grants remain available on
unrooted devices. The launch step depends on baseline; with both permission
steps selected, stable topological ordering keeps baseline, elevated, then
launch in that order.

### 15.4 Legacy compatibility

Package-based permission steps retain the legacy parameter contract and
execution behavior until the designated compatibility cutover. Their action
arrays are not copied into the new-form `permission_plan`.

## 16. Executor Responsibilities

The executor is single-threaded in v1.

It owns:

- runtime ref resolution
- artifact download and cache handling
- ZIP extraction
- file copy mechanics
- typed baseline and elevated permission-operation dispatch
- ADB launch/install/force-stop
- verify checks

## 17. Execution Semantics

### Current baseline semantics

For each step:

1. dependency check
2. constraint check
3. `skip_if`
4. execute
5. verify

### 17.1 Dependencies

Baseline agreed semantics:

- failed dependency blocks downstream execution
- skipped dependency does not automatically block downstream execution
- skipped steps produce no outputs

### 17.2 Constraints

Constraints are hard checks.

#### Capabilities

- missing required capability => fail step

#### Conflicts

- active conflict present => fail step

Constraints are checked before `skip_if`.

### 17.3 Skip

`skip_if` is evaluated immediately before execution.

If matched:

- step becomes skipped
- step does not execute
- step produces no outputs
- verify does not run

### 17.4 Verify

Verify runs only after successful step execution.

V1 verify types:

- `path_exists`
- `file_exists`
- `package_installed`

Verify failure turns the step into failed.

### Approved next hardening

A distinct `BLOCKED` status is approved next to replace the current ambiguity between true skips and downstream dependency fallout:

- if a dependency failed or is blocked, current step becomes `BLOCKED`
- blocked steps do not execute, resolve params, or verify
- skipped dependencies still do not automatically block downstream steps

This hardening is specifically intended to prevent noisy cascades after early failures.

## 18. Artifact Resolution and Caching

### 18.1 Authored abstraction

Recipes express only:

- remote URL
- high-level cache mode:
  - `default`
  - `none`

They do not express:

- cache key algorithm
- HEAD/probe mode
- ETag logic
- extraction command
- transport details

### 18.2 Executor ownership

Artifact cache behavior is executor-owned.

The current intent is:

- planner remains declarative
- executor decides how to materialize and cache the artifact

### 18.3 Download error handling

Current real-world issues showed the need for better typed artifact download failures, especially TLS trust-store failures. Approved next hardening includes:

- `artifact_download_failed`
- `tls_verification_failed`

TLS verification remains strict.

## 19. Extraction Behavior

### 19.1 Supported archive type

V1 extraction is ZIP-only.

### 19.2 Host extraction

Host extraction is the primary grouped/batch path.

### 19.3 Device extraction

Device extraction is supported in a basic form.

Any executor-managed temp paths used for device extraction are executor-internal implementation details and should not become new authored schema unless explicitly added later.

## 20. App-Private Writes

No separate authored schema concept is introduced for "elevated destination" or "root destination."

Instead:

- recipe steps use `copy_files`
- constraints declare generic capabilities; named App Targets derive their
  location-specific write capability during planning
- executor internally chooses the correct write path based on destination path class

Typical examples:

- shared storage paths such as `/sdcard/...`
- app-private paths such as `/data/user/0/<package>/...`

This keeps the schema clean while still allowing app-private core seeding, such as RetroArch cores into `/data/user/0/com.retroarch.aarch64/cores`.

## 21. RetroArch Reference Flow

The RetroArch provisioning flow uses App Definition ownership for its reusable
file sources while keeping installation and procedural setup in the Recipe:

1. keep the Recipe's legacy `remote_file` artifact for the RetroArch APK
2. declare seven frontend archives, thirteen selected core archives, and
   three system-resource archives as named `kind: file` App Definition
   artifacts with HTTPS direct-URL sources
3. reference those App Definition artifacts with the Recipe's existing
   `app_artifact` IDs and retain the three existing `artifact_groups`
4. let the Recipe select cache policy, archive extraction, step dependencies,
   and app-aware copy destinations
5. run:
   - `resolve_artifacts`
   - `install_apk`
   - `launch_app`
   - `wait`
   - `force_stop_app`
   - `grant_permissions`
   - `extract_artifacts` for selected cores and system-resource groups
   - `copy_files` to `/data/user/0/com.retroarch.aarch64/cores`
   - `extract_archive` for frontend assets
   - `copy_files` into the App Definition's named targets
   - optional config copy via `copy_files`
   - final launch

The APK installation behavior remains the legacy Recipe artifact flow. All
other RetroArch archive sources are app-owned, and they use the ordinary
artifact resolution, cache, extraction, and local-path workflow.

## 22. Known Current Gaps / Intentional V1 Limits

These are intentionally left narrow in v1:

- executor is single-threaded
- ZIP-only extraction
- host extraction is the primary grouped path
- device extraction is basic
- legacy copy/push step types may still exist for compatibility
- `manual_requirement` may remain only as compatibility for existing `permissions.manual`
- planner does not emit execution commands
- no heuristic path inference in `copy_files`

## 23. Business Logic Summary

In plain terms:

- recipe authors declare desired state and orchestration
- planner turns that into a normalized execution plan
- executor materializes runtime values as steps run
- artifacts are first-class declared resources
- grouped artifacts allow batching without schema bloat
- step outputs are explicit and typed
- permissions are declarative intent, not embedded ADB commands
- app-private writes are capability-gated, not schema-special-cased
- skipped steps do not magically produce outputs
- verify checks correctness only after successful execution
- runtime failures should be surfaced as real executor errors, not hidden behind planner logic

## 24. Recommended Next Work

The next practical milestones are:

1. executor hardening:
   - `BLOCKED` status
   - better `launch_app`
   - typed TLS/download errors
2. device E2E validation:
   - full RetroArch artifact-based apply
   - optional config absent/present
   - grouped core batch flow
   - frontend/system asset flow
   - app-private copy verification
3. cleanup:
   - deprecate/remove legacy copy/push steps once migration is stable
