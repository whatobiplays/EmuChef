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

    def test_list_step_specs_returns_generic_param_shape_metadata(self) -> None:
        step_specs = step_specs_to_dto()
        json.dumps(step_specs)
        specs_by_type = {spec["type"]: spec for spec in step_specs}

        resolve = specs_by_type["resolve_artifacts"]
        self.assertEqual(
            resolve["params"]["artifacts"]["shape"],
            {
                "kind": "list",
                "itemKind": "string",
                "target": "artifact",
                "ordered": True,
                "unique": True,
                "fields": {},
            },
        )
        self.assertEqual(resolve["params"]["artifact_groups"]["shape"]["target"], "artifact_group")

        extract = specs_by_type["extract_artifacts"]
        self.assertEqual(extract["params"]["artifacts"]["shape"]["target"], "artifact")
        self.assertEqual(extract["params"]["artifact_groups"]["shape"]["target"], "artifact_group")

        grant = specs_by_type["grant_permissions"]
        self.assertEqual(
            grant["params"]["runtime"]["shape"],
            {
                "kind": "list",
                "itemKind": "object",
                "ordered": True,
                "unique": False,
                "fields": {
                    "package_name": {"kind": "string", "required": True, "enumValues": []},
                    "name": {"kind": "string", "required": True, "enumValues": []},
                },
            },
        )
        self.assertEqual(grant["params"]["appops"]["shape"]["fields"]["mode"]["kind"], "string")
        self.assertEqual(
            grant["params"]["policy"]["shape"],
            {
                "kind": "object",
                "ordered": False,
                "unique": False,
                "fields": {
                    "on_failure": {"kind": "string", "required": False, "enumValues": ["warn", "fail"], "default": "warn"},
                    "require_all": {"kind": "boolean", "required": False, "enumValues": [], "default": False},
                },
            },
        )


if __name__ == "__main__":
    unittest.main()
