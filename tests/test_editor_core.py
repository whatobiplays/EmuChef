from __future__ import annotations

from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef.io import load_authored_recipe
from emuchef_editor.app.workspace.service import open_workspace, resolve_authored_root
from emuchef_editor.core.validation.validator_service import ValidatorService
from emuchef_editor.core.yaml.loader import load_recipe_document

from support import base_recipe, build_authored_tree


class EditorCoreTests(unittest.TestCase):
    def test_load_recipe_document_constructs_document(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertEqual(document.working_recipe.id, "example.recipe")
            self.assertEqual(document.authored_root, authored_root.resolve())
            self.assertFalse(document.is_dirty)

    def test_canonical_yaml_orders_top_level_keys_and_emits_explicit_refs(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            recipe_dependencies=["dependency.recipe"],
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "description": "Directory to copy.",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                    "default": None,
                }
            },
            steps=[
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": True,
                    "dependencies": [],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Example"},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")
            emitted = document.to_yaml()

            self.assertLess(emitted.index("description:"), emitted.index("recipe_dependencies:"))
            self.assertLess(emitted.index("recipe_dependencies:"), emitted.index("provides:"))
            self.assertIn("ref: inputs.source_dir", emitted)

    def test_ref_index_includes_inputs_artifacts_steps_and_step_outputs(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "archive_zip": {
                    "type": "remote_file",
                    "url": "https://example.com/archive.zip",
                }
            },
            steps=[
                {
                    "id": "extract_archive",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "artifacts.archive_zip.local_path"}},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertIn("artifacts.archive_zip.local_path", document.ref_index.artifact_refs)
            self.assertIn("steps.extract_archive", document.ref_index.step_refs)
            self.assertIn(
                "steps.extract_archive.outputs.extracted_path",
                document.ref_index.step_output_refs,
            )

    def test_semantic_dirty_tracking_ignores_non_canonical_open_then_resets_after_save(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            recipe_path.write_text(
                "\n".join(
                    [
                        "schema_version: 1",
                        "kind: recipe",
                        "id: example.recipe",
                        "name: Example Recipe",
                        "provides:",
                        "  features:",
                        "    - example_recipe",
                        "recipe_dependencies: []",
                        "inputs: {}",
                        "artifacts: {}",
                        "artifact_groups: {}",
                        "permissions:",
                        "  runtime: []",
                        "  appops: []",
                        "  policy:",
                        "    on_failure: warn",
                        "    require_all: false",
                        "steps:",
                        "  - id: grant",
                        "    type: grant_permissions",
                        "    name: Grant",
                        "    user_toggleable: false",
                        "    dependencies: []",
                        "    constraints:",
                        "      capabilities: []",
                        "      conflicts_with: []",
                        "    verify: []",
                        "description: Example Recipe description.",
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            document = load_recipe_document(recipe_path, authored_root=authored_root)
            self.assertFalse(document.is_dirty)

            document.set_working_recipe(replace(document.working_recipe, name="Updated Recipe"))
            self.assertTrue(document.is_dirty)

            document.save()
            self.assertFalse(document.is_dirty)
            saved_text = recipe_path.read_text(encoding="utf-8")
            self.assertLess(saved_text.index("description:"), saved_text.index("recipe_dependencies:"))
            self.assertIn("name: Updated Recipe", saved_text)

    def test_validator_service_preserves_shared_validation_metadata(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "wait",
                    "type": "wait",
                    "name": "Wait",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 0},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            shared_recipe = load_authored_recipe(recipe_path)
            result = ValidatorService().validate_recipe(shared_recipe, path=recipe_path, authored_root=authored_root)

            diagnostic = next(item for item in result.diagnostics if item.code == "param_contract_violation")
            self.assertEqual(diagnostic.severity, "error")
            self.assertEqual(diagnostic.object_kind, "recipe")
            self.assertEqual(diagnostic.object_id, "example.recipe")
            self.assertEqual(diagnostic.field, "steps[0].params.duration_ms")

    def test_workspace_resolution_accepts_repo_root_or_authored_root(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])

            self.assertEqual(resolve_authored_root(repo_root), authored_root.resolve())
            self.assertEqual(resolve_authored_root(authored_root), authored_root.resolve())

            repo_workspace = open_workspace(repo_root)
            authored_workspace = open_workspace(authored_root)

            self.assertEqual(repo_workspace.authored_root, authored_root.resolve())
            self.assertEqual(authored_workspace.authored_root, authored_root.resolve())
            self.assertEqual(len(repo_workspace.recipe_files), 1)
            self.assertEqual(repo_workspace.recipe_files[0].name, "example_recipe.yaml")


if __name__ == "__main__":
    unittest.main()
