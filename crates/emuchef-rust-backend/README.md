# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend skeleton for the EmuChef config
editor protocol. It is standalone and runnable independently of the Tauri
editor.

Through Phase 6M it implements only:

- `hello`
- `listStepSpecs`
- `emitRecipeYamlFromPath`
- `validateRecipePath`
- sidecar-only `openRecipe`
- sidecar-only `getDocument`
- sidecar-only `saveRecipe`
- sidecar-only `closeDocument`
- sidecar-only `applyRecipeCommand`
- sidecar-only `undo`
- sidecar-only `redo`
- sidecar-only `emitYaml`
- sidecar-only `validate`
- sidecar-only `getRefIndex`
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling
- authored recipe YAML load/emit skeletons for focused migration fixtures
- editor-local authored recipe validation diagnostics for focused Phase 6K
  fixtures
- authoredRoot/catalog-context recipe validation diagnostics for focused Phase
  6L fixtures
- in-memory sidecar document sessions backed by the authored recipe model
- snapshot undo/redo for open document sessions
- the Python-compatible `SetOverviewField` command for recipe `name` and
  `description`
- Python-compatible non-step `applyRecipeCommand` mutations for the currently
  modeled input, artifact, and artifact group command families
- Python-compatible step lifecycle and dependency `applyRecipeCommand`
  mutations for the currently modeled step fields
- Python-compatible step params and advanced internals `applyRecipeCommand`
  mutations for the currently modeled step fields
- generated `RecipeDocumentDto.refIndex` data for the currently modeled
  authored recipe features
- an internal-only, fixture-scoped declarative planner skeleton that emits a
  Python-shaped `PlanningResult`/`ExecutionPlan` for focused Phase 6M tests

It reports only capabilities that are implemented in this crate:

```json
{
  "protocolVersion": 1,
  "capabilities": [
    "listStepSpecs",
    "emitRecipeYamlFromPath",
    "validateRecipePath",
    "openRecipe",
    "getDocument",
    "saveRecipe",
    "closeDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "emitYaml",
    "validate",
    "getRefIndex"
  ]
}
```

Reporting document-session capabilities means the Rust backend supports those
requests in JSONL sidecar mode only. One-shot mode remains stateless and does
not expose persistent document session APIs.

Reporting these capabilities still does not make this backend compatible with
the Tauri editor. Phase 6M reports the same ordered capability list as Phase
6H/6I/6J.1/6J.2/6K/6L; capability parity is not full backend parity. The Rust
backend is still not editor-ready and is not wired into Tauri.
There is no env var, CLI flag, config file, README path, or documented launch
path for using this Rust backend as the Tauri editor backend in Phase 6M.

The Python backend remains the reference implementation. This Rust package is
not a replacement backend and is not selected by the Tauri editor.

This package does not implement full planner behavior, executor behavior,
Python bundling, or production packaging. Phase 6K replaced the earlier basic
validation skeleton with fixture-covered editor-local validation parity for the
current Rust recipe model scope. Phase 6L adds fixture-covered
authoredRoot/catalog-context validation for Python-verified recipe dependency
diagnostics. Phase 6M adds a minimal internal declarative planner skeleton for
focused fixtures only. Rust still does not perform executor work, device checks,
artifact downloads, archive extraction, file copies, permission grants,
subprocess execution, or Tauri integration. Python remains the reference
implementation until parity is confirmed.

## Phase 6M Planner Scope

Phase 6M adds `src/planner.rs`, an internal Rust module used only by crate-local
tests. It emits the Python planner's serialized `PlanningResult` shape with a
nested `ExecutionPlan` for narrow fixtures. The parity target is Python's
`Planner.start_session(...).emit_execution_plan()` path, not CLI summary text.
The current editor protocol does not expose a planner request, so Phase 6M does
not add a one-shot request, JSONL sidecar request, capability string, Tauri
command, TypeScript API, backend selector, or config/env toggle.

The planner input is explicit fixture data: loaded Rust `Recipe` values, selected
recipe refs, a supplied device-plan ref, a supplied device-profile ref, a supplied
`DeviceContext`, supplied `RuntimeCapabilities`, and optional input bindings.
`DeviceContext` is modeled only to serialize the Python `ExecutionPlan` golden
shape. `RuntimeCapabilities` values in tests mirror the Python fixture defaults
from `tests/support.py`; Rust does not resolve profiles, probe devices, or invent
runtime capability defaults.

Planner fixture loading reads only top-level `authored/recipes/*.yml` /
`authored/recipes/*.yaml` files from the Phase 6L-style authoredRoot fixture
trees. It does not scan apps, profiles, device plans, nested recipe directories,
templates, project roots, or runtime filesystem state. Dependency expansion uses
only the loaded recipe metadata and preserves Python's dependency-before-selected
recipe order for the covered fixtures.

Phase 6M planner output parity is functional/semantic. Byte-for-byte JSON output
is not a guarantee, but the tests compare structured `PlanningResult` fields,
execution step order, ids, dependencies, refs, materialized StepSpec defaults,
artifact expansion, input binding errors, and no-side-effect behavior. The Phase
6M JSON fixtures under `tests/fixtures/python_goldens/phase6m_*` are generated
from the actual Python planner API.

Phase 6M does not execute steps. It does not create output directories, mutate
authored YAML, mutate document-session state, copy files, download artifacts,
extract archives, inspect devices, call ADB, grant permissions, run subprocesses,
perform network checks, add production packaging, bundle Python, or replace the
Python backend.

### Built-In Planner Behavior

| Step type | Phase 6M status | Notes |
| --- | --- | --- |
| `wait` | Implemented for fixture coverage | Emits literal params and supports authored/dependency ordering tests. |
| `resolve_artifacts` | Implemented for fixture coverage | Expands `artifacts` and `artifact_groups` into execution artifact ids. Does not download or resolve files. |
| `extract_artifacts` | Implemented for fixture coverage | Expands artifact selections and materializes Python StepSpec default `extract_on: host`. Does not extract archives or touch host/device files. |
| `copy_files` | Implemented for fixture coverage | Normalizes top-level refs, materializes Python StepSpec default `copy_policy: merge`, and emits declarative params only. Does not copy files. |
| `grant_permissions` | Implemented for fixture coverage | Keeps permission params step-local and does not emit a separate permission plan or grant permissions. |
| `extract_archive` | Deferred | Default behavior is inspected but not part of Phase 6M Rust fixtures. No archive extraction is implemented. |
| `install_apk` | Deferred | Ref/default normalization may be added in a later fixture-backed planner phase. No install behavior is implemented. |
| `launch_app` | Deferred | Not needed for Phase 6M fixtures. No app launch behavior is implemented. |
| `force_stop_app` | Deferred | Not needed for Phase 6M fixtures. No device operation is implemented. |

## Document Session Scope

Phase 6J.2 document sessions are process-local JSONL sidecar state. Opening a
recipe loads the Phase 6E authored recipe model, emits canonical YAML, records
that YAML as the saved baseline, and returns a Python-shaped `RecipeDocumentDto`.
`applyRecipeCommand` supports the overview commands added in Phase 6G:

```json
{"type":"SetOverviewField","field":"name","value":"New Name"}
{"type":"SetOverviewField","field":"description","value":"New description"}
{"type":"SetOverviewField","field":"description","value":null}
```

Recipe `id`, `kind`, `schemaVersion`, and `schema_version` are read-only in this
Rust backend slice and are rejected. `description: null` matches Python
behavior: it clears the description, projects the DTO description as an empty
string, and omits the top-level `description:` key from canonical YAML. Empty or
whitespace-only `description` values also clear the field. Empty or
whitespace-only `name` values fail command execution.

Phase 6I expands `applyRecipeCommand` to these non-step command families:

```text
AddInput
RenameInput
UpdateInputField
DeleteInput
DuplicateInput
AddArtifact
UpdateArtifactField
RenameArtifact
DeleteArtifact
DuplicateArtifact
AddArtifactGroup
RenameArtifactGroup
DeleteArtifactGroup
DuplicateArtifactGroup
ReorderArtifactGroup
AddArtifactGroupMember
RemoveArtifactGroupMember
ReorderArtifactGroupMember
```

Input mutations cover the fields currently decoded by Python:

```text
type
role
label
description
required
multiple
validation.must_exist
validation.allowed_extensions
validation.path_kind
```

Artifact mutations cover Python's current authored `remote_file` fixture surface:
`url` and `cache`. Artifact `type` remains authored as `remote_file` in the Rust
model and is not editable through a Phase 6I command. This is fixture-scoped
artifact parity, not a claim of broader artifact kind support.

Phase 6J.1 expands `applyRecipeCommand` to these step lifecycle and dependency
command families:

```text
AddStep
DeleteStep
DuplicateStep
ReorderStep
UpdateStepBasics
SetStepUserToggleable
UpdateStepDependencies
```

`AddStep` creates the same verified authored step shape as Python for this
migration slice: normalized id/name, supported `stepType`, `user_toggleable:
false`, empty `dependencies`, empty `constraints`, empty `skip_if`, empty
`params`, empty `verify`, and no description. It does not materialize StepSpec
default params. `UpdateStepBasics` edits only `name` and `description`; `null`,
empty, and whitespace-only descriptions clear the description, project as an
empty DTO string, and omit `description:` from canonical YAML. Step `id` and
`type` remain read-only.

`UpdateStepDependencies` is a full replacement command. It preserves authored
order, normalizes dependency ids, rejects duplicate dependency ids, and
intentionally allows missing dependency targets because Python command
application allows them. Phase 6K validation reports local missing-target and
cycle diagnostics after the command applies; command application still does not
construct a planner graph or infer execution order.

Phase 6J.2 expands `applyRecipeCommand` to the remaining current external
Python step params and advanced internals command families:

```text
UpdateStepParams
UpdateStepConstraints
UpdateStepSkipIf
UpdateStepVerify
```

`UpdateStepParams` is a full replacement command. It preserves submitted param
object order in the in-memory model, converts only an immediate param value with
the exact JSON shape `{"ref":"..."}` and a string ref into an authored ref, and
keeps nested or list-contained ref-shaped objects literal. It removes builtin
StepSpec default literals only when the authored value compares equal under the
Python command semantics used by the editor. It does not infer missing params,
materialize defaults, validate ref existence, or add dependencies from refs.

`UpdateStepConstraints` is a full replacement command. The JSON command shape
uses `conflictsWith`; emitted authored YAML uses `conflicts_with`; DTOs continue
to use `conflictsWith`. Constraint object keys other than `capabilities` and
`conflictsWith` are rejected as `invalid_command`. Duplicate or blank-after-trim
identifiers decode successfully when they are non-empty JSON strings, then fail
application as `command_failed`, matching the Python command path.

`UpdateStepSkipIf` and `UpdateStepVerify` are full replacement commands for
condition lists. Each condition accepts only `type` and optional `params`;
unknown condition types pass through; `params` defaults to an empty object and
must be an object when present. Ref-shaped values inside condition params remain
literal JSON/YAML values and do not affect dependencies or RefIndex.

The current external Python `command_codec.py` `_DECODERS` inventory contains 30
command types. Phase 6J.2 supports all 30 command type strings through Rust
`applyRecipeCommand`. Internal/core-only Python command dataclasses that are not
present in `_DECODERS` are not external sidecar commands and remain outside this
Rust backend slice: recipe dependency list commands, provided feature list
commands, `RenameRecipeIdCommand`, and `RenameStepCommand`.

Input, artifact, and artifact group rename/delete commands perform the same
tested safe cleanup as Python for supported immediate step params:

- input refs such as `inputs.source_dir`
- artifact field refs such as `artifacts.target_zip.local_path`
- supported `artifacts` string-list params
- supported `artifact_groups` string-list params
- artifact group membership when artifacts are renamed or deleted

`DeleteStep` performs only the Python-verified cleanup targets that the Rust
model already represents faithfully for Phase 6J.1 fixtures: other steps'
`dependencies`, `constraints.conflicts_with`, and supported top-level step param
refs such as `steps.prepare` and `steps.prepare.outputs.extracted_path`.

Nested literal ref-shaped data, `skip_if`, `verify`, unsupported step types, and
custom params remain preserved instead of recursively rewritten. Step delete
does not expand skip/verify or advanced internals cleanup in Phase 6J.2.
Artifact groups are not RefIndex sources; group mutations do not create
`artifact_groups.*` refs.

Changing commands regenerate canonical YAML first, then rerun the Phase 6K
editor-local validation checks for the current in-memory recipe. No-op commands return
`changed: false` and do not push undo history. Invalid commands leave the
stored document unchanged. Successful changing commands also return a refreshed
document DTO, including current dirty/canUndo/canRedo state and a RefIndex
derived from the mutated recipe.

`saveRecipe` writes the current canonical YAML back to the document's current
path, updates the saved baseline after a successful write, reruns Phase 6K
editor-local validation, preserves undo/redo stacks, and returns the current
document DTO. Save tests must use temporary copies of fixtures; do not point save
tests at checked-in fixture files.

Undo/redo use content snapshots of the recipe model and current canonical YAML,
then rerun validation after restoring changed content. The saved baseline is not
part of content snapshots; dirty state is always recalculated from the current
canonical YAML versus the most recent opened/saved baseline. Undo and redo on
empty stacks match Python behavior by returning `changed: false` success
responses with the current document.

`emitYaml` returns the current in-memory canonical YAML for an open document.
`validate` reruns Phase 6K editor-local validation for the current in-memory
recipe and returns diagnostics only.

`getRefIndex` returns generated RefIndex data for the current in-memory document
state. `RecipeDocumentDto.refIndex` is no longer the Phase 6F empty placeholder
for modeled Phase 6H recipe features. The generated index is fixture-scoped and
limited to the currently modeled Rust authored recipe data: inputs, runtime
artifact fields, authored step ids, and declared StepSpec outputs from the
embedded Python StepSpec fixture. It does not derive planner, catalog-context,
executor/device, artifact group, recipe `provides`, or missing-param refs.

```json
{
  "inputRefs": ["inputs.bios_source_dir"],
  "artifactRefs": [],
  "stepRefs": ["steps.copy_bios_dir"],
  "stepOutputRefs": ["steps.copy_bios_dir.outputs.copied_paths"],
  "allRefs": [
    "inputs.bios_source_dir",
    "steps.copy_bios_dir",
    "steps.copy_bios_dir.outputs.copied_paths"
  ],
  "candidates": [
    {
      "ref": "inputs.bios_source_dir",
      "label": "Input · bios_source_dir",
      "valueType": "directory_path",
      "sourceKind": "input",
      "sourceId": "bios_source_dir"
    }
  ]
}
```

## StepSpec Source

`listStepSpecs` returns a static parity copy of the current Python built-in step
spec metadata. The fixture lives at:

```text
crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json
```

That fixture is temporary Phase 6D scaffolding, not the final Rust step
registry. Later phases should replace it with Rust-native schema builders before
planner or executor behavior is ported. Regenerate and compare the fixture any
time Python step specs change.

Regenerate the fixture from the repo root with:

```bash
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python -m emuchef_editor.api.server '{"type":"listStepSpecs"}' \
  | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["result"], indent=2, sort_keys=True))' \
  > crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json
```

The fixture stores only the Python response `result` object, not the outer
`{"ok": true, "result": ...}` envelope.

## Authored YAML Scope

`emitRecipeYamlFromPath` loads an authored recipe YAML file, builds the narrow
Phase 6E Rust model, and emits canonical authored YAML. Semantic YAML parity with
Python is the priority. Byte-for-byte parity is best effort and is only asserted
where stable; broader fixture tests compare parsed YAML semantics so harmless
emitter formatting does not block the phase.

The Rust loader supports these authored recipe sections:

- `schema_version`
- `kind`
- `id`
- `name`
- `description`
- `recipe_dependencies`
- `provides`
- `inputs`
- `artifacts`
- `artifact_groups`
- `steps`

Top-level `permissions` is rejected. Permission intent remains step-local through
`grant_permissions.params`.

Step params treat a value as a ref only when the immediate param value is a map
with exactly one key, `ref`, and that value is a string. Nested ref-shaped maps in
lists, objects, `skip_if`, or `verify` remain literal authored data.

`validateRecipePath` returns the Python-compatible result shape:

```json
{"diagnostics":[]}
```

For Phase 6K and Phase 6L fixtures it targets functional/semantic parity with Python
editor-facing diagnostics. Structured diagnostic fields are the primary parity
contract: `severity`, `code`, `objectKind`, `objectId`, `field`, and diagnostic
presence. Byte-for-byte `message` equality is best effort and not required when
the message has the same meaning.

Phase 6K editor-local validation covers only data available from the loaded
recipe, embedded StepSpecs, and local authored refs:

- load/schema shape diagnostics for the current Rust YAML model
- removed top-level `permissions`
- local authored step dependency missing-target and cycle diagnostics
- StepSpec missing required params and literal/ref mode checks
- exact top-level `ParamValue::Ref` validation for inputs, artifacts, artifact
  runtime fields, steps, and primary step outputs
- nested ref-shaped literals in params, `skip_if`, and `verify` remaining
  literal data

Known Phase 6K limits:

- Malformed YAML parser messages come from `serde_yaml`, so wording and source
  spans can differ from PyYAML. The diagnostic shape, severity, code, file, and
  object fields are still matched.
- Planner and executor validation remain unimplemented.
- Broad built-in plugin hook validation is not complete. Phase 6K implements only
  fixture-required local checks; omitted plugin-hook diagnostics remain future
  work.
- Python remains the reference implementation until parity is confirmed. The
  project direction is a hard Rust cutover after parity, not a user-facing
  backend selector or long-term dual-backend toggle.

Phase 6L authoredRoot/catalog-context validation covers the Python-verified
recipe diagnostics needed by the focused Rust fixtures:

- `validateRecipePath` uses an explicit non-null `authoredRoot` as catalog
  context and does not infer it from the recipe path. Omitted `authoredRoot` and
  `authoredRoot: null` keep the `validation_context_limited` warning.
- `openRecipe` normalizes an explicit repo root containing `authored/recipes` to
  that `authored` directory, accepts an explicit `authored` directory, and infers
  `authoredRoot` from recipe files under an `authored/recipes` tree.
- sidecar `validate`, successful changing commands, undo, redo, and save reuse
  the document's stored authoredRoot. They do not re-infer a root after open.
- recipe dependency missing-target diagnostics use `recipe_not_found`.
- recipe dependency cycles use a small validation-local graph walk and report
  `dependency_cycle`; Rust does not introduce planner module naming, planner
  data structures, execution plans, step expansion, or apply-device behavior.
- duplicate recipe ids between the open document and another catalog recipe use
  `recipe_id_conflict`.

Phase 6L intentionally reads only this Python-verified top-level catalog
inventory:

```text
apps/*.y*ml
recipes/*.y*ml
device_profiles/*.y*ml
device_plans/*.y*ml
```

The Rust Phase 6L fixture implementation models only `recipes/*.yml` and
`recipes/*.yaml` data because no focused fixture needs app, device profile, or
device plan diagnostics. It still confines catalog discovery to the verified
top-level directory inventory and does not scan nested directories, templates,
alternate file extensions, runtime paths, device paths, network URLs, or Tauri
workspace metadata. Cross-file `provides.features` availability is not
implemented because Python validation does not emit provided-feature diagnostics
in this path. Duplicate recipe-id diagnostics are fixture-scoped to opened
recipes under the authored catalog root; temp copies opened outside that root do
not report catalog duplicate-id conflicts so earlier document-session fixture
flows remain stable.

## Python Goldens

Recipe fixtures live under:

```text
crates/emuchef-rust-backend/tests/fixtures/recipes/
```

Focused Phase 6L authoredRoot fixture trees live under:

```text
crates/emuchef-rust-backend/tests/fixtures/authored_root/
```

Python-generated parity fixtures live under:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/
```

Regenerate the Phase 6E goldens from the repo root with:

```bash
PYTHONPATH=src uv run --no-project --native-tls --with PyYAML python - <<'PY'
from __future__ import annotations
import json
from pathlib import Path
from emuchef_editor.api.server import handle_request

fixtures = Path("crates/emuchef-rust-backend/tests/fixtures")
recipes = fixtures / "recipes"
goldens = fixtures / "python_goldens"
goldens.mkdir(parents=True, exist_ok=True)

emit_names = ["minimal_recipe", "representative_recipe", "ref_params"]
validate_names = [
    "minimal_recipe",
    "invalid_top_level_permissions",
    "unsupported_step_type",
    "malformed",
]

for name in emit_names:
    response = handle_request({
        "type": "emitRecipeYamlFromPath",
        "payload": {"path": str(recipes / f"{name}.yaml"), "authoredRoot": None},
    })
    if not response["ok"]:
        raise SystemExit(json.dumps(response, indent=2))
    (goldens / f"{name}.emit.yaml").write_text(response["result"]["yaml"], encoding="utf-8")

for name in validate_names:
    response = handle_request({
        "type": "validateRecipePath",
        "payload": {"path": str(recipes / f"{name}.yaml"), "authoredRoot": None},
    })
    if not response["ok"]:
        raise SystemExit(json.dumps(response, indent=2))
    (goldens / f"{name}.validate.json").write_text(
        json.dumps(response["result"], indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
PY
```

The emitted YAML goldens store only the Python result string. The validation
goldens store only the Python result object, not the outer API envelope.
Phase 6L adds Python-generated diagnostic goldens named
`phase6l_*.diagnostics.json`. They store the semantic diagnostic fields the Rust
tests compare: severity, code, objectKind, objectId, and field.
Phase 6M adds `phase6m_planner_*.json` planning-result fixtures in this
directory. They are generated from Python `Planner(...).start_session(...).emit_execution_plan()`
using the focused Phase 6M authoredRoot recipe fixtures.

Regenerate the Phase 6L diagnostic goldens from the repo root with:

```bash
PYTHONPATH=tests .venv/bin/python - <<'PY'
from __future__ import annotations

import json
from pathlib import Path

from emuchef_editor.api.server import handle_request
from emuchef_editor.api.session import DocumentSessionManager

ROOT = Path("crates/emuchef-rust-backend/tests/fixtures/authored_root")
GOLDENS = Path("crates/emuchef-rust-backend/tests/fixtures/python_goldens")
GOLDENS.mkdir(parents=True, exist_ok=True)

def workspace_root(name: str) -> Path:
    return ROOT / name

def authored_root(name: str) -> Path:
    return workspace_root(name) / "authored"

def recipe_path(workspace: str, name: str) -> Path:
    return authored_root(workspace) / "recipes" / name

def diagnostic_fields(diagnostic: dict) -> dict:
    return {
        "severity": diagnostic.get("severity"),
        "code": diagnostic.get("code"),
        "objectKind": diagnostic.get("objectKind"),
        "objectId": diagnostic.get("objectId"),
        "field": diagnostic.get("field"),
    }

def write(name: str, diagnostics: list[dict]) -> None:
    payload = [diagnostic_fields(item) for item in diagnostics]
    (GOLDENS / name).write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

def validate(path: Path, root_marker) -> list[dict]:
    payload = {"path": str(path)}
    if root_marker != "omitted":
        payload["authoredRoot"] = None if root_marker is None else str(root_marker)
    response = handle_request({"type": "validateRecipePath", "payload": payload})
    if not response["ok"]:
        raise SystemExit(json.dumps(response, indent=2))
    return response["result"]["diagnostics"]

def open_document(path: Path, root_marker="omitted") -> list[dict]:
    manager = DocumentSessionManager()
    authored = None if root_marker == "omitted" else root_marker
    response = manager.open_recipe(path, authored_root=authored)
    if not response["ok"]:
        raise SystemExit(json.dumps(response, indent=2))
    return response["result"]["document"]["diagnostics"]

write("phase6l_complete_null_root.diagnostics.json", validate(recipe_path("complete", "main.yaml"), None))
write("phase6l_complete_explicit_root.diagnostics.json", validate(recipe_path("complete", "main.yaml"), authored_root("complete")))
write("phase6l_missing_authored_root.diagnostics.json", validate(recipe_path("complete", "main.yaml"), workspace_root("complete") / "missing-authored-root"))
write("phase6l_missing_dependency.diagnostics.json", validate(recipe_path("missing_dependency", "missing_dependency.yaml"), authored_root("missing_dependency")))
write("phase6l_dependency_cycle.diagnostics.json", validate(recipe_path("dependency_cycle", "cycle_a.yaml"), authored_root("dependency_cycle")))
write("phase6l_nested_ignored.diagnostics.json", validate(recipe_path("nested_ignored", "main.yaml"), authored_root("nested_ignored")))
write("phase6l_duplicate_open.diagnostics.json", open_document(recipe_path("duplicate", "target_duplicate.yaml"), authored_root("duplicate")))
write("phase6l_duplicate_reverse_open.diagnostics.json", open_document(recipe_path("duplicate_reverse", "a_target_duplicate.yaml"), authored_root("duplicate_reverse")))
write("phase6l_missing_dependency_open.diagnostics.json", open_document(recipe_path("missing_dependency", "missing_dependency.yaml"), "omitted"))
PY
```

Regenerate the Phase 6M planner goldens from the repo root with:

```bash
PYTHONPATH=tests .venv/bin/python - <<'PY'
from __future__ import annotations

import json
import tempfile
from pathlib import Path

import yaml

from emuchef.domain import DeviceContext
from emuchef.io import load_authored_catalog
from emuchef.io.serde import to_primitive
from emuchef.planner import Planner
from support import build_authored_tree

ROOT = Path("crates/emuchef-rust-backend/tests/fixtures/authored_root")
GOLDENS = Path("crates/emuchef-rust-backend/tests/fixtures/python_goldens")
GOLDENS.mkdir(parents=True, exist_ok=True)

CASES = [
    ("planner_minimal", ["main.yaml"], ["planner.minimal"], "phase6m_planner_minimal.json", None),
    ("planner_dependencies", ["dependency.yaml", "main.yaml"], ["planner.dependencies"], "phase6m_planner_dependencies.json", None),
    ("planner_refs_artifacts", ["main.yaml"], ["planner.refs_artifacts"], "phase6m_planner_refs_artifacts.json", None),
    ("planner_inputs", ["main.yaml"], ["planner.inputs"], "phase6m_planner_inputs_bound.json", {"planner.inputs/required_cfg": "/tmp/example.cfg"}),
    ("planner_inputs", ["main.yaml"], ["planner.inputs"], "phase6m_planner_inputs_missing.json", None),
    ("planner_grant_permissions", ["main.yaml"], ["planner.grant_permissions"], "phase6m_planner_grant_permissions.json", None),
]

for fixture, recipe_files, selected_recipe_refs, golden_name, bindings in CASES:
    recipes = []
    for recipe_file in recipe_files:
        with (ROOT / fixture / "authored" / "recipes" / recipe_file).open(encoding="utf-8") as handle:
            recipes.append(yaml.safe_load(handle))

    with tempfile.TemporaryDirectory() as tmp:
        authored_root = build_authored_tree(
            Path(tmp),
            recipes=recipes,
            selected_recipe_refs=selected_recipe_refs,
        )
        session = Planner(load_authored_catalog(authored_root)).start_session(
            "example.device_plan",
            DeviceContext(
                manufacturer="Example",
                model="Example",
                android_version=13,
                android_api_level=33,
                device_tags=(),
            ),
        )
        if bindings:
            for input_id, value in bindings.items():
                update = session.bind_input(input_id, value)
                if update.errors:
                    raise RuntimeError(update.errors)

        result = to_primitive(session.emit_execution_plan())
        (GOLDENS / golden_name).write_text(
            json.dumps(result, indent=2, sort_keys=False) + "\n",
            encoding="utf-8",
        )
PY
```

Phase 6G document sessions are covered by Rust integration tests and focused
Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6g_*.result.json
```

Those goldens cover overview name changes, description changes,
`description:null`, no-op commands, empty undo/redo, undo/redo after mutation,
open-document `emitYaml`, and open-document `validate`. They normalize
`documentId`, paths, authored roots, and diagnostic files. The Phase 6G golden
recipe has no inputs, artifacts, or steps, so its generated RefIndex is empty
even after Phase 6H replaces the former placeholder behavior for richer
modeled recipes.

Regenerate the Phase 6G goldens from the repo root with:

```bash
PYTHONPATH=src:tests uv run --no-project --native-tls --with PyYAML python - <<'PY'
from __future__ import annotations
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from support import build_authored_tree
from emuchef_editor.api.session import DocumentSessionManager

fixtures = Path("crates/emuchef-rust-backend/tests/fixtures")
goldens = fixtures / "python_goldens"
goldens.mkdir(parents=True, exist_ok=True)

recipe = {
    "schema_version": 1,
    "kind": "recipe",
    "id": "phase6g.golden",
    "name": "Phase 6G Golden",
    "description": "Original description.",
    "recipe_dependencies": [],
    "provides": {"features": []},
    "inputs": {},
    "artifacts": {},
    "artifact_groups": {},
    "steps": [],
}

def normalize(value):
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if key == "documentId":
                out[key] = "<documentId>"
            elif key == "path":
                out[key] = "<path>"
            elif key == "authoredRoot":
                out[key] = "<authoredRoot>" if item is not None else None
            elif key == "file":
                out[key] = "<path>" if item is not None else None
            else:
                out[key] = normalize(item)
        return out
    if isinstance(value, list):
        return [normalize(item) for item in value]
    return value

with TemporaryDirectory() as tmp:
    root = build_authored_tree(Path(tmp), recipes=[recipe])
    path = root / "recipes" / "phase6g_golden.yaml"
    manager = DocumentSessionManager()
    opened = manager.open_recipe(path, authored_root=root)
    document_id = opened["result"]["document"]["documentId"]

    responses = {
        "phase6g_empty_undo.result.json": manager.undo(document_id)["result"],
        "phase6g_empty_redo.result.json": manager.redo(document_id)["result"],
        "phase6g_set_overview_name.result.json": manager.apply_recipe_command(
            document_id,
            {"type": "SetOverviewField", "field": "name", "value": "Python Golden Name"},
        )["result"],
        "phase6g_set_overview_description.result.json": manager.apply_recipe_command(
            document_id,
            {"type": "SetOverviewField", "field": "description", "value": "Python Golden Description"},
        )["result"],
        "phase6g_set_overview_description_null.result.json": manager.apply_recipe_command(
            document_id,
            {"type": "SetOverviewField", "field": "description", "value": None},
        )["result"],
        "phase6g_set_overview_noop.result.json": manager.apply_recipe_command(
            document_id,
            {"type": "SetOverviewField", "field": "name", "value": "Python Golden Name"},
        )["result"],
        "phase6g_undo_after_overview.result.json": manager.undo(document_id)["result"],
        "phase6g_redo_after_overview.result.json": manager.redo(document_id)["result"],
        "phase6g_emit_yaml_after_overview.result.json": manager.emit_yaml(document_id)["result"],
        "phase6g_validate_after_overview.result.json": manager.validate(document_id)["result"],
    }

for filename, result in responses.items():
    (goldens / filename).write_text(
        json.dumps(normalize(result), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
PY
```

Phase 6H RefIndex parity is covered by focused Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6h_*.result.json
```

Those goldens cover a representative document open result, sidecar
`getRefIndex`, overview-only mutation, undo/redo after mutation, and a
ref-parameter fixture `getRefIndex` result. They normalize `documentId`, paths,
authored roots, and diagnostic files. The `ref_params` open-document result is
not used as a full-document golden because Python validation continues to be the
reference beyond the fixture-scoped Rust validation surface; the Phase 6H
comparison for that fixture is intentionally scoped to `getRefIndex`.

Regenerate the Phase 6H goldens from the repo root with:

```bash
PYTHONPATH=src:tests uv run --no-project --native-tls --with PyYAML python - <<'PY'
from __future__ import annotations
import json
from pathlib import Path
from emuchef_editor.api.session import DocumentSessionManager

fixtures = Path("crates/emuchef-rust-backend/tests/fixtures")
recipes = fixtures / "recipes"
goldens = fixtures / "python_goldens"
goldens.mkdir(parents=True, exist_ok=True)

def normalize(value):
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if key == "documentId":
                out[key] = "<documentId>"
            elif key == "path":
                out[key] = "<path>"
            elif key == "authoredRoot":
                out[key] = "<authoredRoot>" if item is not None else None
            elif key == "file":
                out[key] = "<path>" if item is not None else None
            else:
                out[key] = normalize(item)
        return out
    if isinstance(value, list):
        return [normalize(item) for item in value]
    return value

def write(name, value):
    (goldens / name).write_text(
        json.dumps(normalize(value), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

manager = DocumentSessionManager()
opened = manager.open_recipe(recipes / "representative_recipe.yaml", authored_root=fixtures)
if not opened["ok"]:
    raise SystemExit(json.dumps(opened, indent=2))
document_id = opened["result"]["document"]["documentId"]
write("phase6h_representative_open.result.json", opened["result"])
write("phase6h_representative_get_ref_index.result.json", manager.get_ref_index(document_id)["result"])
write(
    "phase6h_representative_set_overview.result.json",
    manager.apply_recipe_command(
        document_id,
        {"type": "SetOverviewField", "field": "name", "value": "Phase 6H Renamed"},
    )["result"],
)
write("phase6h_representative_undo.result.json", manager.undo(document_id)["result"])
write("phase6h_representative_redo.result.json", manager.redo(document_id)["result"])

manager = DocumentSessionManager()
opened = manager.open_recipe(recipes / "ref_params.yaml", authored_root=fixtures)
if not opened["ok"]:
    raise SystemExit(json.dumps(opened, indent=2))
document_id = opened["result"]["document"]["documentId"]
write("phase6h_ref_params_get_ref_index.result.json", manager.get_ref_index(document_id)["result"])
PY
```

Phase 6I non-step command RefIndex parity is covered by focused
Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6i_*.result.json
```

Those goldens intentionally compare `getRefIndex` results after input, artifact,
and artifact group mutations. They do not compare full document results for the
Phase 6I fixture because Python performs richer catalog-context validation than
the Rust backend's current fixture-scoped validation surface.

Regenerate the Phase 6I goldens from the repo root with:

```bash
PYTHONPATH=src:tests uv run --no-project --native-tls --with PyYAML python - <<'PY'
from __future__ import annotations
import json
from pathlib import Path
from emuchef_editor.api.session import DocumentSessionManager

fixtures = Path("crates/emuchef-rust-backend/tests/fixtures")
recipes = fixtures / "recipes"
goldens = fixtures / "python_goldens"
goldens.mkdir(parents=True, exist_ok=True)

def write(name, value):
    (goldens / name).write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

def opened_manager():
    manager = DocumentSessionManager()
    opened = manager.open_recipe(recipes / "phase6i_commands.yaml", authored_root=fixtures)
    if not opened["ok"]:
        raise SystemExit(json.dumps(opened, indent=2))
    return manager, opened["result"]["document"]["documentId"]

manager, document_id = opened_manager()
for command in [
    {"type": "AddInput", "inputId": "gold_input"},
    {"type": "RenameInput", "inputId": "source_dir", "newInputId": "src_dir"},
    {"type": "DuplicateInput", "sourceInputId": "bios_file", "newInputId": "bios_copy"},
    {"type": "DeleteInput", "inputId": "gold_input"},
]:
    result = manager.apply_recipe_command(document_id, command)
    if not result["ok"]:
        raise SystemExit(json.dumps(result, indent=2))
write("phase6i_after_inputs_get_ref_index.result.json", manager.get_ref_index(document_id)["result"])

manager, document_id = opened_manager()
for command in [
    {"type": "AddArtifact", "artifactId": "new_zip", "url": "https://example.com/new.zip"},
    {"type": "RenameArtifact", "artifactId": "target_zip", "newArtifactId": "renamed_zip"},
    {"type": "DeleteArtifact", "artifactId": "renamed_zip"},
]:
    result = manager.apply_recipe_command(document_id, command)
    if not result["ok"]:
        raise SystemExit(json.dumps(result, indent=2))
write("phase6i_after_artifacts_get_ref_index.result.json", manager.get_ref_index(document_id)["result"])

manager, document_id = opened_manager()
for command in [
    {"type": "AddArtifactGroup", "groupId": "gold_group"},
    {"type": "AddArtifactGroupMember", "groupId": "gold_group", "artifactId": "other_zip"},
    {"type": "RenameArtifactGroup", "groupId": "bundle", "newGroupId": "renamed_bundle"},
    {"type": "DeleteArtifactGroup", "groupId": "renamed_bundle"},
]:
    result = manager.apply_recipe_command(document_id, command)
    if not result["ok"]:
        raise SystemExit(json.dumps(result, indent=2))
write("phase6i_after_groups_get_ref_index.result.json", manager.get_ref_index(document_id)["result"])
PY
```

Phase 6J.1 and Phase 6J.2 do not add Python-generated result goldens. Their Rust
tests mirror the verified Python command codec and document behavior directly,
with comments or test names covering non-obvious rules such as missing
dependency target allowance, `AddStep` empty param initialization, description
clearing, delete-step cleanup, params-only ref lifting, StepSpec default
omission, constraints application failures, and literal condition params.

## One-Shot Mode

Run one request as a single JSON argument:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"hello"}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"listStepSpecs"}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"emitRecipeYamlFromPath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}'
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- '{"type":"validateRecipePath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}'
```

Expected `hello` stdout is one JSON response envelope:

```json
{"ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate","getRefIndex"]}}
```

`listStepSpecs` returns `{"stepSpecs":[...]}` inside the success envelope.
`emitRecipeYamlFromPath` returns `{"yaml":"..."}` inside the success envelope.
`validateRecipePath` returns `{"diagnostics":[...]}` inside the success envelope.

## JSONL Sidecar Mode

Run the sidecar loop with `--sidecar`:

```bash
printf '%s\n' '{"id":"hello-1","type":"hello","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"specs-1","type":"listStepSpecs","payload":{}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"emit-1","type":"emitRecipeYamlFromPath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
printf '%s\n' '{"id":"validate-1","type":"validateRecipePath","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Expected `hello` stdout is one JSON response line:

```json
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate","getRefIndex"]}}
```

Session APIs are sidecar-only. A practical manual smoke is:

1. Start an interactive sidecar:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

2. Paste an `openRecipe` JSON line, then copy the returned `documentId` into
   `applyRecipeCommand`, `undo`, `redo`, `emitYaml`, `validate`, `getRefIndex`,
   `saveRecipe`, `getDocument`, or `closeDocument` JSON lines before sending EOF. Use a
   temporary copy of a fixture if the smoke includes `saveRecipe`:

```json
{"id":"open-1","type":"openRecipe","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}
```

The automated Rust integration tests keep the sidecar process alive and cover
`openRecipe`, `getDocument`, `applyRecipeCommand`, `undo`, `redo`, `emitYaml`,
`validate`, `getRefIndex`, `saveRecipe`, and `closeDocument` without adding any production
test-helper command.

Request-level errors are returned as API envelopes and do not terminate the
sidecar:

```bash
printf '%s\n%s\n' '{"id":"bad","type":"unknown"}' '{"id":"specs-2","type":"listStepSpecs"}' | cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

Mixed sidecar and one-shot CLI usage is a process-level usage error:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar '{"type":"hello"}'
```

That command exits non-zero, writes the usage error to stderr, and emits no API
envelope on stdout.

## Validation

Run the crate tests with:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
```
