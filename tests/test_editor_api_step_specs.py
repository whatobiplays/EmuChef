from __future__ import annotations

import json
import unittest

from emuchef_editor.api.dto import step_specs_to_dto


class EditorApiStepSpecTests(unittest.TestCase):
    def test_list_step_specs_returns_stable_json_safe_shapes(self) -> None:
        step_specs = step_specs_to_dto()

        json.dumps(step_specs)
        specs_by_type = {spec["type"]: spec for spec in step_specs}
        copy_files = specs_by_type["copy_files"]

        self.assertEqual(copy_files["label"], "Copy Files")
        self.assertTrue(copy_files["supported"])
        self.assertEqual(copy_files["primaryOutputName"], "copied_paths")
        self.assertEqual(copy_files["paramOrder"], ["source", "dest", "copy_policy"])
        self.assertEqual(
            copy_files["refFilters"]["source"],
            ["file_path", "directory_path", "path_list"],
        )
        self.assertIn(
            {"name": "copied_paths", "valueType": "path_list", "primary": True},
            copy_files["outputs"],
        )


if __name__ == "__main__":
    unittest.main()
