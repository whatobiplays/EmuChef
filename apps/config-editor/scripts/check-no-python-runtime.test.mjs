import assert from "node:assert/strict";
import test from "node:test";

import { findForbiddenRuntimeHits } from "./check-no-python-runtime.mjs";

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
