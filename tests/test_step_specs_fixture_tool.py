from __future__ import annotations

import contextlib
import importlib.util
import io
import json
from argparse import Namespace
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = REPO_ROOT / "scripts" / "step_specs_fixture.py"


def load_tool_module():
    spec = importlib.util.spec_from_file_location("step_specs_fixture_tool_under_test", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("step_specs_fixture.py should be importable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class StepSpecsFixtureToolTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tool = load_tool_module()

    def test_canonical_generated_json_shape_includes_step_specs_root(self) -> None:
        text = self.tool.canonical_json_text(self.tool.generate_fixture_obj())
        parsed = json.loads(text)

        self.assertEqual(list(parsed), ["stepSpecs"])
        self.assertIsInstance(parsed["stepSpecs"], list)
        self.assertGreater(len(parsed["stepSpecs"]), 0)
        self.assertTrue(text.endswith("\n"))

    def test_generated_json_is_stable_across_repeated_calls(self) -> None:
        first = self.tool.canonical_json_text(self.tool.generate_fixture_obj())
        second = self.tool.canonical_json_text(self.tool.generate_fixture_obj())

        self.assertEqual(first, second)

    def test_check_succeeds_against_matching_temp_fixture(self) -> None:
        current = self.tool.canonical_json_text(self.tool.generate_fixture_obj())
        with TemporaryDirectory() as tmp:
            fixture = Path(tmp) / "python_step_specs.json"
            fixture.write_text(current, encoding="utf-8")

            stdout = io.StringIO()
            stderr = io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exit_code = self.tool.cmd_check(Namespace(fixture=fixture))

            self.assertEqual(exit_code, 0)
            self.assertIn("StepSpec fixture check: unchanged", stdout.getvalue())
            self.assertEqual(stderr.getvalue(), "")
            self.assertEqual(fixture.read_text(encoding="utf-8"), current)

    def test_check_fails_on_drift_without_mutating_fixture(self) -> None:
        stale = '{"stepSpecs": []}\n'
        with TemporaryDirectory() as tmp:
            fixture = Path(tmp) / "python_step_specs.json"
            fixture.write_text(stale, encoding="utf-8")

            stdout = io.StringIO()
            stderr = io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exit_code = self.tool.cmd_check(Namespace(fixture=fixture))

            self.assertNotEqual(exit_code, 0)
            self.assertIn("Regenerate with:", stdout.getvalue())
            self.assertIn("StepSpec fixture check: drift detected", stderr.getvalue())
            self.assertIn("---", stderr.getvalue())
            self.assertIn("+++", stderr.getvalue())
            self.assertEqual(fixture.read_text(encoding="utf-8"), stale)

    def test_write_creates_or_updates_out_of_date_temp_fixture(self) -> None:
        expected = self.tool.canonical_json_text(self.tool.generate_fixture_obj())
        with TemporaryDirectory() as tmp:
            fixture = Path(tmp) / "nested" / "python_step_specs.json"

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = self.tool.cmd_write(Namespace(fixture=fixture))

            self.assertEqual(exit_code, 0)
            self.assertIn("StepSpec fixture write: updated", stdout.getvalue())
            self.assertEqual(fixture.read_text(encoding="utf-8"), expected)

    def test_write_reports_unchanged_when_temp_fixture_is_current(self) -> None:
        current = self.tool.canonical_json_text(self.tool.generate_fixture_obj())
        with TemporaryDirectory() as tmp:
            fixture = Path(tmp) / "python_step_specs.json"
            fixture.write_text(current, encoding="utf-8")

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = self.tool.cmd_write(Namespace(fixture=fixture))

            self.assertEqual(exit_code, 0)
            self.assertIn("StepSpec fixture write: unchanged", stdout.getvalue())
            self.assertEqual(fixture.read_text(encoding="utf-8"), current)

    def test_script_imports_and_generation_do_not_load_pyside_or_app_package(self) -> None:
        before_pyside = {name for name in sys.modules if name == "PySide6" or name.startswith("PySide6.")}
        before_app = {name for name in sys.modules if name == "emuchef_editor.app" or name.startswith("emuchef_editor.app.")}

        self.tool.generate_fixture_obj()

        after_pyside = {name for name in sys.modules if name == "PySide6" or name.startswith("PySide6.")}
        after_app = {name for name in sys.modules if name == "emuchef_editor.app" or name.startswith("emuchef_editor.app.")}

        self.assertEqual(after_pyside, before_pyside)
        self.assertEqual(after_app, before_app)


if __name__ == "__main__":
    unittest.main()
