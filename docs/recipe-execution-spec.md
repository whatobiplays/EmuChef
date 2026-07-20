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

`artifacts` is a map keyed by artifact ID.

V1 artifact type:

- `remote_file`

Fields:

- `type`: required, must be `remote_file`
- `url`: required
- `cache`: optional
  - `default`
  - `none`

Example:

```yaml
artifacts:
  retroarch_apk:
    type: remote_file
    url: https://buildbot.libretro.com/nightly/android/RetroArch_aarch64.apk
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

Permissions remain top-level and declarative.

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
- `dest`: required literal device path
- `copy_policy`: optional, default `merge`
  - `merge`
  - `replace`
  - `sync`

Source may resolve to:

- `file_path`
- `directory_path`
- `path_list`

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

Params:

- none

Behavior:

- implicitly consumes the permission plan
- valid even when there are no local permission declarations or no applicable actions
- succeeds cleanly as a no-op in that case

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

### 15.1 Planner output

The permission plan contains structured intent only.

It does not contain:

- ADB command tuples
- shell command details

Permission plan actions carry fields such as:

- action kind
- package name
- permission/op/manual type
- requiredness
- source metadata
- reason
- status

### 15.2 Action statuses

- `applicable`
- `not_applicable`
- `manual`

### 15.3 Action kinds

Minimal v1 kinds:

- `runtime_permission`
- `appop`

Compatibility exception:

- existing `manual_requirement` may remain only if needed to represent `permissions.manual`

### 15.4 `grant_permissions` execution logic

Executor behavior:

- execute applicable actions
- record `not_applicable`
- record `manual`
- no-op cleanly when no actions apply

Policy behavior:

- applicable `required=true` failures are hard failures
- `require_all=true` upgrades any applicable failure to hard failure
- otherwise non-required failures may warn/continue per policy

## 16. Executor Responsibilities

The executor is single-threaded in v1.

It owns:

- runtime ref resolution
- artifact download and cache handling
- ZIP extraction
- file copy mechanics
- runtime permission command generation
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
- constraints declare required capability such as `app_data_write`
- executor internally chooses the correct write path based on destination path class

Typical examples:

- shared storage paths such as `/sdcard/...`
- app-private paths such as `/data/user/0/<package>/...`

This keeps the schema clean while still allowing app-private core seeding, such as RetroArch cores into `/data/user/0/com.retroarch.aarch64/cores`.

## 21. RetroArch Reference Flow

The intended RetroArch flow now looks like this conceptually:

1. declare top-level remote artifacts for:
   - RetroArch APK
   - frontend assets ZIP
   - selected system ZIPs
   - selected core ZIPs
2. declare grouped core selection in `artifact_groups`
3. run:
   - `resolve_artifacts`
   - `install_apk`
   - `launch_app`
   - `wait`
   - `force_stop_app`
   - `grant_permissions`
   - `extract_artifacts` for selected cores
   - `copy_files` to `/data/user/0/com.retroarch.aarch64/cores`
   - `extract_archive` or `extract_artifacts` for frontend/system assets
   - `copy_files` into destination directories
   - optional config copy via `copy_files`
   - final launch

This replaces the older local-sample-artifact model with a real artifact-centric provisioning path.

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
