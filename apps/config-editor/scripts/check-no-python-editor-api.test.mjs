import assert from "node:assert/strict";
import test from "node:test";

import {
  findForbiddenPythonEditorApiHits,
  parsePythonEditorApiState,
} from "./check-no-python-editor-api.mjs";

test("pyproject parser allows ordinary CLI scripts", () => {
  const state = parsePythonEditorApiState(`
[project.scripts]
emuchef = "emuchef.cli:main"
`);

  assert.equal(state.hasPublishedEditorApiScript, false);
});

test("pyproject parser rejects editor API entrypoints", () => {
  const state = parsePythonEditorApiState(`
[project.scripts]
emuchef = "emuchef.cli:main"
emuchef-editor-api = "emuchef_editor.api.server:main"
`);

  assert.equal(state.hasPublishedEditorApiScript, true);
});

test("guard flags editor API source paths and active Python imports", () => {
  assert.deepEqual(findForbiddenPythonEditorApiHits("src/emuchef_editor/api/server.py", ""), [
    { filePath: "src/emuchef_editor/api/server.py", token: "Python editor API source path", line: 0 },
  ]);
  assert.deepEqual(
    findForbiddenPythonEditorApiHits(
      "tests/test_editor_api_server.py",
      "from emuchef_editor.api.server import handle_request\n",
    ),
    [{ filePath: "tests/test_editor_api_server.py", token: "emuchef_editor.api", line: 1 }],
  );
});

test("guard allows documentation references", () => {
  assert.deepEqual(
    findForbiddenPythonEditorApiHits("README.md", "python -m emuchef_editor.api.server"),
    [],
  );
});
