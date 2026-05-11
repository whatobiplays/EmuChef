# EmuChef Rust Backend Skeleton

This package is an experimental Rust backend skeleton for the EmuChef config
editor protocol. It is standalone and runnable independently of the Tauri
editor.

Phase 6E implements only:

- `hello`
- `listStepSpecs`
- `emitRecipeYamlFromPath`
- `validateRecipePath`
- stable success and error envelopes
- structured API errors
- one-shot JSON request handling
- JSON Lines sidecar request handling
- authored recipe YAML load/emit skeletons for focused Phase 6E fixtures
- basic authored recipe validation diagnostics for focused Phase 6E fixtures

It reports only capabilities that are implemented in this crate:

```json
{
  "protocolVersion": 1,
  "capabilities": ["listStepSpecs", "emitRecipeYamlFromPath", "validateRecipePath"]
}
```

Reporting these capabilities does not make this backend compatible with the
Tauri editor. The current Tauri compatibility gate still rejects it because
document/session/editor capabilities such as `openRecipe`, `getDocument`,
`applyRecipeCommand`, `undo`, `redo`, `saveRecipe`, `validate`, `emitYaml`, and
`getRefIndex` are missing.

The Python backend remains the reference implementation. This Rust package is
not a replacement backend and is not selected by the Tauri editor.

This package does not create document sessions, apply editor commands, save
files, build a ref index, run planner behavior, run executor behavior, bundle
Python, or provide production packaging. Its validation is a basic skeleton only;
it does not perform full catalog-context validation, dependency graph validation,
planner contract validation, artifact expansion validation, device checks, or
executor checks.

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
{"ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath"]}}
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
{"id":"hello-1","ok":true,"result":{"protocolVersion":1,"capabilities":["listStepSpecs","emitRecipeYamlFromPath","validateRecipePath"]}}
```

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
