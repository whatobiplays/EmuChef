# Runtime Recipe Configuration

## 1. Purpose and ownership

Recipe inputs are EmuChef's public runtime-configuration surface. The Rust
runtime is the sole implementation of this contract. Recipe authors
declare what can be configured, saved user-configuration documents persist user
choices without changing recipe YAML, and planning resolves those choices into a
normalized execution plan.

The ownership boundary is:

1. Authored recipes declare inputs and consume them through recipe-local refs.
2. Device plans may provide device-specific input overrides.
3. User configurations persist recipe selection and direct input bindings.
4. Planning requests may replace selection or values for one run.
5. The planner resolves precedence, validates effective values, and emits typed
   plan inputs.
6. The executor consumes the normalized plan. It does not load configurations,
   merge layers, prompt for values, or interpret provenance.

User customization never patches or clones an authored recipe.

## 2. Recipe input declarations

Inputs are an ordered mapping under a recipe's `inputs` field. The current ROM
copy recipe is the canonical authored example:

```yaml
inputs:
  source:
    type: directory
    role: rom_library
    label: ROM source folder
    required: true
    multiple: false
    validation:
      must_exist: true
      allowed_extensions: []
      path_kind: directory
    default: null
  destination:
    type: device_path
    role: rom_destination
    label: Device ROM folder
    required: true
    multiple: false
    validation:
      must_exist: false
      allowed_extensions: []
      path_kind: directory
      allowed_prefixes:
        - /sdcard
        - /storage/emulated/0
    default: /sdcard/ROMs
  policy:
    type: enum
    role: copy_policy
    label: Copy policy
    required: false
    multiple: false
    validation:
      must_exist: false
      allowed_extensions: []
    default: merge
    options:
      - value: merge
        label: Merge files
      - value: replace
        label: Replace destination
      - value: sync
        label: Mirror source
```

First-class fields are:

1. `type`: the authored input type.
2. `role`: an optional semantic purpose such as `rom_library`.
3. `label`: the human-readable control label.
4. `description`: optional explanatory text.
5. `required`: whether planning requires a value after precedence is applied.
6. `multiple`: whether the value is a list of the declared scalar type.
7. `default`: a JSON-compatible recipe default, or `null` when absent.
8. `validation`: structured value constraints.
9. `options`: ordered enum values and presentation labels.
10. `sensitive`: diagnostic and logging redaction metadata.
11. `advanced`: presentation metadata for controls normally hidden in a basic
    view.
12. `metadata`: an extension object for information that does not belong in a
    first-class field.

`advanced` does not change selection, precedence, validation, or execution.
Enum labels are presentation-only; binding and planning use option values.

## 3. Input and runtime value types

The supported authored types and their planner runtime representation are:

| Authored input type | Runtime value type | JSON shape |
| --- | --- | --- |
| `string` | `string` | string |
| `integer` | `integer` | integer |
| `boolean` | `boolean` | boolean |
| `enum` | `string` | declared option value |
| `file` | `file_path` | string path |
| `directory` | `directory_path` | string path |
| `path` | `path` | string path |
| `device_path` | `device_path` | string path |
| `string_list` | `string_list` | string array |
| `path_list` | `path_list` | string-path array |
| `object` | `object` | JSON object |

`multiple: true` changes a scalar declaration to a list of its scalar values.
CLI list values must be JSON arrays; repeated `--bind` keys are not list syntax.

## 4. Validation phases

Validation is divided by the information available at each phase:

1. Recipe loading validates declaration structure, supported types, default
   shape, unique enum options, enum defaults, and coherent validation fields.
2. User-configuration loading validates only document structure and does not
   require an authored catalog.
3. Standalone user-configuration validation uses a supplied authored root to
   report catalog-dependent semantic diagnostics.
4. `describeConfiguration` and `planConfiguration` resolve precedence and
   validate only the winning value for each known effective input.
5. The executor handles runtime concerns such as actual device writability.

Host `file` and `directory` values may be checked for existence and path kind
when planning runs on the host. `allowed_extensions` constrains applicable host
path values. `allowed_prefixes` constrains `device_path` strings without
rewriting them. Device writability is not inferred from a string prefix.

For discovery and planning, a valid higher-precedence value can shadow an
invalid saved value. The shadowed saved value does not fail that request.
Standalone validation still reports the saved value's semantic problem so it
can be edited.

## 5. Refs and qualified binding keys

Recipe-authored refs remain local to the recipe:

```yaml
steps:
  - id: copy_rom_library
    type: copy_files
    params:
      source:
        ref: inputs.source
      dest:
        ref: inputs.destination
      copy_policy:
        ref: inputs.policy
```

The planner can normalize these refs internally, but recipe authors do not write
qualified binding keys as step refs.

Configuration layers use fully qualified keys:

```text
<recipe-id>/<input-id>
```

For example:

```text
feature.copy_roms/source
feature.copy_roms/destination
feature.copy_roms/policy
```

Unqualified global input IDs are not supported.

## 6. Precedence and provenance

For each input in the dependency-expanded selected recipe set, the planner uses
the first present value in this order:

1. Explicit planning-request binding, provenance `explicit`.
2. Persisted user-configuration binding, provenance `user_configuration`.
3. Device-plan input override, provenance `device_plan`.
4. Recipe input default, provenance `recipe_default`.
5. Unbound, which is an error for a required input and valid for an optional
   input.

The input maps remain separate until the centralized resolver chooses a winner.
An invalid winner is an error and does not fall back to a lower layer. Unknown
keys and keys outside the effective dependency-expanded recipe set are errors.
Device-plan metadata such as configuration variants, UI defaults, and advanced
visibility is not an input binding.

Provenance is returned by discovery and planning. It is not passed to the
executor as a new configuration responsibility.

## 7. Persisted user-configuration schema

Schema version 1 uses this canonical shape:

```yaml
schema_version: 1
kind: user_configuration
id: example.pocket-s-mini.roms
name: Pocket S Mini ROM Setup
device_plan: ayaneo.pocket_s_mini.base
selected_recipes:
  - feature.copy_roms
bindings:
  feature.copy_roms/destination:
    value: /sdcard/Emulation/ROMs
  feature.copy_roms/policy:
    value: sync
  feature.copy_roms/source:
    value: /Users/example/Emulation/ROMs
```

The complete example is
[`examples/user-configurations/example.pocket-s-mini.roms.yaml`](../../examples/user-configurations/example.pocket-s-mini.roms.yaml).

`schema_version`, `kind`, `id`, `name`, `device_plan`, `selected_recipes`, and
`bindings` are required. `device_plan` must be a non-empty string. A persisted
document remains self-contained even when a request-level `devicePlan` replaces
its saved value for discovery or planning.

Each binding entry is strict and contains exactly one value-source field. Schema
version 1 implements only direct `value` bindings. Zero fields, multiple source
fields, unsupported source fields, and unknown binding-entry fields are
structural errors.

Structurally safe unknown top-level extension fields are preserved. Canonical
known fields always win and are emitted first, extensions cannot replace or
duplicate them, and extension keys have deterministic ordering. Unknown fields
inside binding entries are not preserved.

### 7.1 Structural loading failures

The following prevent loading:

1. Malformed YAML or duplicate YAML mapping keys.
2. Unsupported `schema_version` or `kind`.
3. Missing, null, empty, or incorrectly typed required top-level fields,
   including `device_plan`.
4. Malformed `<recipe-id>/<input-id>` binding-key syntax.
5. A binding entry with zero or multiple value-source fields.

### 7.2 Catalog-aware semantic diagnostics

The following do not prevent a structurally valid document from loading,
editing, or canonical emission:

1. Missing referenced device plan.
2. Unknown selected or bound recipe.
3. Unknown input.
4. A binding outside the dependency-expanded selected recipe set.
5. Incompatible binding value type.
6. Invalid enum value or path prefix.
7. Missing required input.

`validateUserConfigurationPath` validates a standalone file. An opened editor
session uses `validateUserConfiguration`. Supplying an authored root enables
the catalog-aware checks; parsing and emission remain catalog-independent.

## 8. Configuration roots and ID-or-path resolution

An identifier resolves directly to:

```text
<configuration-root>/<id>.yaml
```

The configuration root means the directory containing user-configuration
documents. EmuChef never appends another `user_configurations` directory and
never searches the authored root or current working directory.

The defaults are:

1. macOS: `~/Library/Application Support/EmuChef/user-configurations`
2. Windows: `%APPDATA%\EmuChef\user-configurations`
3. Linux with XDG: `$XDG_CONFIG_HOME/emuchef/user-configurations`
4. Linux fallback: `~/.config/emuchef/user-configurations`

`--configuration-root` and the protocol `configurationRoot` field provide a
deterministic override for tests, portable installations, and automation.

ID-or-path classification uses syntax only:

1. Absolute values are paths.
2. Values containing `/` or `\` are paths.
3. Values ending in `.yaml` or `.yml`, using ASCII case-insensitive matching,
   are paths.
4. Other values are identifiers and must match
   `[A-Za-z0-9][A-Za-z0-9._-]*`.

A path remains a path when it does not exist. Failed path loading never falls
back to identifier resolution. Explicit relative and absolute YAML paths are
accepted by `--user-configuration` and protocol requests.

## 9. CLI planning

Plan with explicit selection and bindings:

```bash
emuchef plan \
  --authored-root authored \
  --device-plan ayaneo.pocket_s_mini.base \
  --recipe feature.copy_roms \
  --bind feature.copy_roms/source=/Volumes/ROMs \
  --bind feature.copy_roms/destination=/sdcard/Emulation/ROMs \
  --bind feature.copy_roms/policy=sync \
  --output /tmp/emuchef-plan.yaml
```

Plan from a saved identifier under an explicit root:

```bash
emuchef plan \
  --authored-root authored \
  --configuration-root "$HOME/.config/emuchef/user-configurations" \
  --user-configuration example.pocket-s-mini.roms \
  --output /tmp/emuchef-plan.yaml
```

Override one saved value for the run:

```bash
emuchef plan \
  --authored-root authored \
  --configuration-root "$HOME/.config/emuchef/user-configurations" \
  --user-configuration example.pocket-s-mini.roms \
  --bind feature.copy_roms/source=/Volumes/Travel-ROMs \
  --output /tmp/emuchef-plan.yaml
```

Each qualified key may occur at most once in `--bind`. The value is parsed as
JSON when valid and otherwise preserved as a string. Lists and `multiple: true`
values therefore use a JSON array:

```bash
--bind 'feature.example/extensions=["zip","7z"]'
```

Two occurrences of `--bind feature.copy_roms/policy=...` are a structured CLI
error and are never converted into an array.

The JSON protocol enforces the same one-value-per-qualified-key rule at its raw
request boundary. For `describeConfiguration` and `planConfiguration`, a
duplicate key in the request `bindings` object, or in an inline configuration's
`bindings` object, fails before conversion to a JSON map:

```json
{
  "ok": false,
  "error": {
    "code": "invalid_request",
    "message": "Request field 'bindings' contains a duplicate key.",
    "details": {
      "reason": "duplicate_binding_key",
      "field": "bindings",
      "key": "feature.copy_roms/policy"
    }
  }
}
```

The error contains the qualified key but neither duplicate value. This raw JSON
rule is distinct from CLI `--bind` argument parsing.

Recipe selection has three states:

1. No `--recipe` and no `--clear-recipes` leaves the explicit selection absent,
   so saved or device-plan selection applies.
2. One or more `--recipe ID` occurrences provide a replacement selection.
3. `--clear-recipes` provides an explicit empty replacement.

Combining `--clear-recipes` with any `--recipe` is a structured CLI error.

## 10. Discovery protocol

`describeConfiguration` accepts a device plan or saved configuration plus
optional request replacements:

```json
{
  "type": "describeConfiguration",
  "payload": {
    "authoredRoot": "/path/to/authored",
    "configurationRoot": "/path/to/user-configurations",
    "userConfiguration": "example.pocket-s-mini.roms",
    "devicePlan": "ayaneo.pocket_s_mini.base",
    "selectedRecipes": ["feature.copy_roms"],
    "bindings": {
      "feature.copy_roms/destination": "/sdcard/Games"
    },
    "deviceContext": {}
  }
}
```

The surrounding protocol uses camelCase request fields. `userConfiguration`
accepts either an ID/path string or an inline document object. An inline object
uses the canonical persisted document schema and therefore keeps snake_case
document fields:

```json
{
  "type": "describeConfiguration",
  "payload": {
    "authoredRoot": "/path/to/authored",
    "userConfiguration": {
      "schema_version": 1,
      "kind": "user_configuration",
      "id": "example.config",
      "name": "Example",
      "device_plan": "ayaneo.pocket_s_mini.base",
      "selected_recipes": ["feature.copy_roms"],
      "bindings": {
        "feature.copy_roms/destination": {
          "value": "/sdcard/ROMs"
        }
      }
    }
  }
}
```

Inline documents are structurally parsed by the same schema-v1 parser as YAML
documents, do not perform file lookup, and do not accept camelCase aliases for
document fields.

An explicitly present `devicePlan` replaces the saved `device_plan`. An
explicitly present `selectedRecipes` replaces saved or device-plan selection;
`[]` is an explicit empty replacement. An absent field inherits the lower
layer.

The result contains effective and dependency-expanded recipe IDs, one ordered
entry per effective input, declaration metadata, partial effective values,
provenance, and diagnostics. Missing required inputs do not prevent discovery.
Discovery performs no ADB probing, downloads, extraction, host copies, device
writes, or persistence.

## 11. Planning protocol

`planConfiguration` accepts the same context as discovery. It performs complete
catalog, dependency, winning-binding, ref, planning-time constraint, and
required-value validation. A successful envelope contains a structured result:

```json
{
  "ok": true,
  "result": {
    "plan": {
      "id": "plan.ayaneo.pocket_s_mini.base.001",
      "steps": []
    },
    "resolvedInputs": [],
    "diagnostics": []
  }
}
```

When planning diagnostics prevent a valid plan, the operation still returns its
structured result with `plan: null` and the diagnostics. Malformed requests and
load failures use the existing error envelope.

`planConfiguration` is deliberately side-effect-free. It performs no ADB
commands, downloads, extraction, host copies, device writes, plan-file writes,
or execution. Saving returned plan JSON is a separate frontend or future
explicit persistence action.

Product callers provide a resolved `catalog` snapshot containing local root,
source kind/id, optional version/cache key, and optional content integrity
digest. `authoredRoot` remains the legacy compatibility adapter. Catalog
identity/version and content integrity are separate concepts; `cached_remote`
is reserved and has no networking implementation.

Product planning may include reviewed `targetDevice` facts. A successful result
also contains `planDigest`, computed from canonical JSON SHA-256. The plan
captures catalog identity, target binding, ordered recipe display snapshots,
and ordered step notes. `startExecution` requires the digest and recomputes it
before accepting an attempt. See
[Phase 0 End-User Runtime Contracts](../product/phase-0-runtime-contracts.md)
for the canonical JSON, target matching, and report definitions.

## 12. Executor boundary

Execution plans retain normalized typed input values so refs such as
`inputs.source`, `inputs.destination`, and `inputs.policy` resolve without
guessing every value from a raw string. Step parameters retain the canonical ref
form where the plan contract expects refs. The executor resolves those values
from execution state, then performs the step.

The executor does not know whether a value originated in a recipe, device plan,
saved configuration, or explicit request. It does not fall back between layers.

## 13. Portability, sensitive values, and compatibility

Host paths are local to the machine that plans and executes the configuration.
The example `/Users/example/Emulation/ROMs` must be changed on hosts where that
directory does not exist. Device paths are Android paths and remain unchanged.
For portable automation, use an explicit configuration root and supply
machine-local host paths as explicit bindings.

`sensitive: true` prevents diagnostics and logs from including the supplied
value or a serialized representation. Diagnostics report only the qualified
key, expected type or constraint, and provenance. This flag is not encryption,
secret storage, access control, or automatic masking of configuration files and
structured plan/discovery values. Schema version 1 stores direct values in
plain YAML; secrets should not be persisted there.

Existing recipes without the new presentation metadata remain supported when
their declarations are otherwise valid. Canonical recipe YAML continues to use
direct literals and single-field `{ ref: ... }` mappings. Existing literal-only
or ref-only step parameters retain their source restrictions; the typed source
contract does not make arbitrary refs valid. Frozen compatibility goldens are
not regenerated for this feature. New runtime-configuration behavior is covered
by Rust-native tests and the current authored corpus.

Recipe steps may provide an optional `progress_note` for the end-user execution
surface. Its absence does not change schema validity or execution semantics.
The runtime falls back to step name, humanized step type, then step id. Only
representative checked-in recipes need authored notes; deterministic fallback
keeps the rest of the catalog usable without a bulk migration.
