import assert from "node:assert/strict";
import test from "node:test";

import {
  findForbiddenRuntimeHits,
  productRuntimeContractErrors,
} from "./check-no-python-runtime.mjs";

test("accepts the canonical Rust CLI and legacy-only Python entrypoint contract", () => {
  const errors = productRuntimeContractErrors({
    pyproject: '[project.scripts]\nemuchef-python-legacy = "emuchef.cli:main"\n',
    cargoManifest: '[package]\ndefault-run = "emuchef"\n[[bin]]\nname = "emuchef"\npath = "src/main.rs"\n',
    tauriConfig: '{"bundle":{"externalBin":["binaries/emuchef"]}}',
    packaging: 'export const BINARY_BASENAME = "emuchef";',
  });

  assert.deepEqual(errors, []);
});

test("rejects a Python-owned emuchef entrypoint and v1 sidecar naming", () => {
  const errors = productRuntimeContractErrors({
    pyproject: '[project.scripts]\nemuchef = "emuchef.cli:main"\n',
    cargoManifest: '[package]\ndefault-run = "emuchef-rust-backend"\n',
    tauriConfig: '{"bundle":{"externalBin":["binaries/emuchef-rust-backend"]}}',
    packaging: 'export const BINARY_BASENAME = "emuchef-rust-backend";',
  });

  assert.ok(errors.includes("Python-owned emuchef console entrypoint"));
  assert.ok(errors.includes("missing emuchef-python-legacy console entrypoint"));
  assert.ok(errors.some((error) => error.includes("Cargo default-run emuchef")));
  assert.ok(errors.some((error) => error.includes("Tauri emuchef externalBin")));
});

test("flags forbidden runtime command tokens and explicit Python module names", () => {
  const hits = findForbiddenRuntimeHits(
    "example.rs",
    'Command::new("python").arg("-m").arg("emuchef_editor.api.server");\nconst bridge = "python_bridge";',
  );

  assert.deepEqual(
    hits.map((hit) => hit.token),
    ["python", "emuchef_editor.api.server", "python_bridge"],
  );
});

test("does not flag forbidden token substrings inside unrelated words", () => {
  const hits = findForbiddenRuntimeHits(
    "example.json",
    '{"scripts":{"dev":"uvicorn app:main && echo pythonista && echo not_python3 && echo python_bridge_extra"}}',
  );

  assert.deepEqual(hits, []);
});

test("flags Windows executable command tokens", () => {
  const hits = findForbiddenRuntimeHits(
    "example.json",
    '{"scripts":{"bad":"python.exe && python3.exe && uv.exe"}}',
  );

  assert.deepEqual(
    hits.map((hit) => hit.token),
    ["python.exe", "python3.exe", "uv.exe"],
  );
});

test("ignores forbidden tokens inside Rust cfg(test) modules", () => {
  const hits = findForbiddenRuntimeHits(
    "example.rs",
    'fn runtime_command() {}\n\n#[cfg(test)]\nmod tests {\n    const FORBIDDEN: [&str; 3] = ["python", "python3", "uv"];\n}\n',
  );

  assert.deepEqual(hits, []);
});

test("continues scanning runtime code after Rust cfg(test) helper items", () => {
  const hits = findForbiddenRuntimeHits(
    "example.rs",
    'impl Client {\n    #[cfg(test)]\n    fn helper() { const TOKEN: &str = "python"; }\n}\n\nfn runtime_command() { Command::new("python"); }\n\n#[cfg(test)]\nmod tests {\n    const TOKEN: &str = "uv";\n}\n',
  );

  assert.deepEqual(
    hits.map((hit) => hit.token),
    ["python"],
  );
  assert.equal(hits[0].line, 6);
});
