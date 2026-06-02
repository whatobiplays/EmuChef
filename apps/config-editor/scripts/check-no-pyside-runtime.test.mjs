import assert from "node:assert/strict";
import test from "node:test";

import {
  findForbiddenPySideHits,
  parsePyProjectQuarantineState,
} from "./check-no-pyside-runtime.mjs";

test("pyproject quarantine parser allows no PySide dependency or editor script", () => {
  const state = parsePyProjectQuarantineState(`
[project]
dependencies = ["PyYAML>=6.0"]

[project.scripts]
emuchef = "emuchef.cli:main"

[tool.setuptools]
include-package-data = false

[tool.setuptools.packages.find]
where = ["src"]
`);

  assert.deepEqual(state.basePySideDependencies, []);
  assert.deepEqual(state.optionalPySideDependencies, []);
  assert.equal(state.hasPublishedEditorScript, false);
  assert.equal(state.disablesImplicitPackageData, true);
});

test("pyproject quarantine parser flags base and optional PySide dependencies plus editor script", () => {
  const state = parsePyProjectQuarantineState(`
[project]
dependencies = ["PySide6>=6.8", "PyYAML>=6.0"]

[project.optional-dependencies]
pyside-editor = ["PySide6>=6.8"]

[project.scripts]
emuchef = "emuchef.cli:main"
emuchef-editor = "emuchef_editor.app.app:main"

[tool.setuptools.packages.find]
where = ["src"]
`);

  assert.deepEqual(state.basePySideDependencies, ["PySide6>=6.8"]);
  assert.deepEqual(state.optionalPySideDependencies, ["PySide6>=6.8"]);
  assert.equal(state.hasPublishedEditorScript, true);
  assert.equal(state.disablesImplicitPackageData, false);
});

test("flags PySide and legacy app imports in active Python paths", () => {
  const hits = findForbiddenPySideHits(
    "src/emuchef_editor/core/example.py",
    [
      "from PySide6.QtWidgets import QApplication",
      "from emuchef_editor.app.workspace.service import open_workspace",
    ].join("\n"),
  );

  assert.deepEqual(
    hits.map((hit) => hit.token),
    ["PySide6", "emuchef_editor.app"],
  );
});

test("flags Qt-specific names in supported PySide-free metadata modules", () => {
  const hits = findForbiddenPySideHits(
    "src/emuchef_editor/core/metadata/tooltips.py",
    "def describe(widget: QtWidgets.QWidget) -> None: pass",
  );

  assert.deepEqual(
    hits.map((hit) => hit.token),
    ["QtWidgets"],
  );
});

test("flags PySide imports in formerly legacy source and tests", () => {
  assert.deepEqual(
    findForbiddenPySideHits("src/emuchef_editor/app/main_window.py", "from PySide6.QtWidgets import QWidget"),
    [
      { filePath: "src/emuchef_editor/app/main_window.py", token: "legacy PySide source path", line: 0 },
      { filePath: "src/emuchef_editor/app/main_window.py", token: "PySide6", line: 1 },
    ],
  );
  assert.deepEqual(
    findForbiddenPySideHits("tests/legacy/test_pyside_editor_app.py", "from PySide6.QtWidgets import QWidget"),
    [
      { filePath: "tests/legacy/test_pyside_editor_app.py", token: "legacy PySide test path", line: 0 },
      { filePath: "tests/legacy/test_pyside_editor_app.py", token: "PySide6", line: 1 },
    ],
  );
});

test("flags formerly legacy PySide paths even without imports", () => {
  assert.deepEqual(
    findForbiddenPySideHits("src/emuchef_editor/app/__init__.py", ""),
    [{ filePath: "src/emuchef_editor/app/__init__.py", token: "legacy PySide source path", line: 0 }],
  );
  assert.deepEqual(
    findForbiddenPySideHits("tests/legacy/__init__.py", ""),
    [{ filePath: "tests/legacy/__init__.py", token: "legacy PySide test path", line: 0 }],
  );
});

test("ignores documentation references", () => {
  const hits = findForbiddenPySideHits(
    "docs/pyside6-retirement-plan.md",
    "The legacy PySide6 editor lives under src/emuchef_editor/app.",
  );

  assert.deepEqual(hits, []);
});
