# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend skeleton for the EmuChef config
editor protocol. It is standalone and runnable independently of the Tauri
editor.

Phase 6G implements only:

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
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling
- authored recipe YAML load/emit skeletons for focused Phase 6E fixtures
- basic authored recipe validation diagnostics for focused Phase 6E fixtures
- in-memory sidecar document sessions backed by the authored recipe model
- snapshot undo/redo for open document sessions
- the Python-compatible `SetOverviewField` command for recipe `name` and
  `description`

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
    "validate"
  ]
}
```

Reporting document-session capabilities means the Rust backend supports those
requests in JSONL sidecar mode only. One-shot mode remains stateless and does
not expose persistent document session APIs.

Reporting these capabilities still does not make this backend compatible with
the Tauri editor. The current Tauri compatibility gate still rejects it because
`getRefIndex` is missing.

The Python backend remains the reference implementation. This Rust package is
not a replacement backend and is not selected by the Tauri editor.

This package does not implement input commands, artifact commands, artifact
group commands, step lifecycle commands, step dependency commands, step params
commands, advanced internals commands, safe-delete behavior, a real ref index,
planner behavior, executor behavior, Python bundling, or production packaging.
Its validation is a basic skeleton only; it does not perform full
catalog-context validation, dependency graph validation, planner contract
validation, artifact expansion validation, device checks, or executor checks.
Python remains the reference implementation.

## Document Session Scope

Phase 6G document sessions are process-local JSONL sidecar state. Opening a
recipe loads the Phase 6E authored recipe model, emits canonical YAML, records
that YAML as the saved baseline, and returns a Python-shaped `RecipeDocumentDto`.
`applyRecipeCommand` supports only:

```json
{"type":"SetOverviewField","field":"name","value":"New Name"}
{"type":"SetOverviewField","field":"description","value":"New description"}
{"type":"SetOverviewField","field":"description","value":null}
```

Recipe `id`, `kind`, `schemaVersion`, and `schema_version` are read-only in this
Phase 6G slice and are rejected. `description: null` matches Python behavior:
it clears the description, projects the DTO description as an empty string, and
omits the top-level `description:` key from canonical YAML. Empty or
whitespace-only `description` values also clear the field. Empty or
whitespace-only `name` values fail command execution.

Changing commands regenerate canonical YAML first, then rerun the Phase 6E basic
validation skeleton for the current in-memory recipe. No-op commands return
`changed: false` and do not push undo history. Invalid commands leave the
stored document unchanged.

`saveRecipe` writes the current canonical YAML back to the document's current
path, updates the saved baseline after a successful write, reruns the Phase 6E
basic validation skeleton, preserves undo/redo stacks, and returns the current
document DTO. Save tests must use temporary copies of fixtures; do not point save
tests at checked-in fixture files.

Undo/redo use content snapshots of the recipe model, current canonical YAML, and
diagnostics. The saved baseline is not part of content snapshots; dirty state is
always recalculated from the current canonical YAML versus the most recent
opened/saved baseline. Undo and redo on empty stacks match Python behavior by
returning `changed: false` success responses with the current document.

`emitYaml` returns the current in-memory canonical YAML for an open document.
`validate` reruns the Phase 6E basic validation skeleton for the current
in-memory recipe and returns diagnostics only.

`refIndex` is a temporary empty Python-compatible placeholder:

```json
{
  "inputRefs": [],
  "artifactRefs": [],
  "stepRefs": [],
  "stepOutputRefs": [],
  "allRefs": [],
  "candidates": []
}
```

The placeholder is structural only. It does not derive refs, candidates, or
partial ref data, and `getRefIndex` is not reported as a capability.

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

For Phase 6E fixtures it mirrors Python diagnostic codes/messages where
practical. Known skeleton differences:

- Malformed YAML parser messages come from `serde_yaml`, so wording and source
  spans can differ from PyYAML. The diagnostic shape, severity, code, file, and
  object fields are still matched.
- Supplying a non-null `authoredRoot` is accepted for payload compatibility, but
  Rust does not load catalogs or perform cross-file validation in Phase 6E.

## Python Goldens

Recipe fixtures live under:

```text
crates/emuchef-rust-backend/tests/fixtures/recipes/
```

Python-generated goldens live under:

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

Phase 6G document sessions are covered by Rust integration tests and focused
Python-generated result goldens:

```text
crates/emuchef-rust-backend/tests/fixtures/python_goldens/phase6g_*.result.json
```

Those goldens cover overview name changes, description changes,
`description:null`, no-op commands, empty undo/redo, undo/redo after mutation,
open-document `emitYaml`, and open-document `validate`. They normalize
`documentId`, paths, authored roots, and diagnostic files. Python currently
returns a populated ref index for richer recipes, while Phase 6G intentionally
returns an empty structural placeholder until the Rust ref index is ported.

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
{"ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate"]}}
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
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath","openRecipe","getDocument","saveRecipe","closeDocument","applyRecipeCommand","undo","redo","emitYaml","validate"]}}
```

Session APIs are sidecar-only. A practical manual smoke is:

1. Start an interactive sidecar:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

2. Paste an `openRecipe` JSON line, then copy the returned `documentId` into
   `applyRecipeCommand`, `undo`, `redo`, `emitYaml`, `validate`, `saveRecipe`,
   `getDocument`, or `closeDocument` JSON lines before sending EOF. Use a
   temporary copy of a fixture if the smoke includes `saveRecipe`:

```json
{"id":"open-1","type":"openRecipe","payload":{"path":"crates/emuchef-rust-backend/tests/fixtures/recipes/minimal_recipe.yaml","authoredRoot":null}}
```

The automated Rust integration tests keep the sidecar process alive and cover
`openRecipe`, `getDocument`, `applyRecipeCommand`, `undo`, `redo`, `emitYaml`,
`validate`, `saveRecipe`, and `closeDocument` without adding any production
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
