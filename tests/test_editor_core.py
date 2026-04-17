from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef.io import load_authored_recipe
from emuchef_editor.app.workspace.service import open_workspace, resolve_authored_root
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddInputCommand,
    AddProvidedFeatureCommand,
    AddRecipeDependencyCommand,
    DeleteArtifactCommand,
    DeleteInputCommand,
    DuplicateArtifactCommand,
    DuplicateInputCommand,
    MoveProvidedFeatureCommand,
    MoveRecipeDependencyCommand,
    RemoveArtifactGroupMemberCommand,
    RemoveProvidedFeatureCommand,
    RemoveRecipeDependencyCommand,
    ReorderArtifactGroupCommand,
    ReorderArtifactGroupMemberCommand,
    SetOverviewFieldCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdateProvidedFeatureCommand,
    UpdateRecipeDependencyCommand,
)
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

            document.apply_command(SetOverviewFieldCommand(field="name", value="Updated Recipe"))
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

    def test_overview_commands_update_fields_lists_and_live_yaml(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertFalse(document.apply_command(SetOverviewFieldCommand(field="name", value="Example Recipe")))
            self.assertFalse(document.is_dirty)

            self.assertTrue(document.apply_command(SetOverviewFieldCommand(field="id", value="updated.recipe")))
            self.assertTrue(document.apply_command(SetOverviewFieldCommand(field="name", value="Updated Recipe")))
            self.assertTrue(document.apply_command(SetOverviewFieldCommand(field="description", value="Updated description.")))
            self.assertTrue(document.apply_command(AddRecipeDependencyCommand(value="dependency.one")))
            self.assertTrue(document.apply_command(AddRecipeDependencyCommand(value="dependency.two")))
            self.assertTrue(document.apply_command(UpdateRecipeDependencyCommand(index=1, value="dependency.renamed")))
            self.assertTrue(document.apply_command(MoveRecipeDependencyCommand(index=1, to_index=0)))
            self.assertTrue(document.apply_command(AddProvidedFeatureCommand(value="secondary_feature")))
            self.assertTrue(document.apply_command(UpdateProvidedFeatureCommand(index=1, value="renamed_feature")))
            self.assertTrue(document.apply_command(MoveProvidedFeatureCommand(index=1, to_index=0)))
            self.assertTrue(document.apply_command(RemoveRecipeDependencyCommand(index=1)))
            self.assertTrue(document.apply_command(RemoveProvidedFeatureCommand(index=1)))

            self.assertEqual(document.working_recipe.id, "updated.recipe")
            self.assertEqual(document.working_recipe.name, "Updated Recipe")
            self.assertEqual(document.working_recipe.description, "Updated description.")
            self.assertEqual(document.working_recipe.recipe_dependencies, ("dependency.renamed",))
            self.assertEqual(document.working_recipe.provides.features, ("renamed_feature",))
            self.assertIn("id: updated.recipe", document.to_yaml())
            self.assertIn("- dependency.renamed", document.to_yaml())
            self.assertIn("- renamed_feature", document.to_yaml())

    def test_input_commands_support_crud_duplicate_and_preserve_extra_fields(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "bios_source_dir": {
                    "type": "directory",
                    "role": "bios",
                    "label": "BIOS Folder",
                    "description": "Folder containing BIOS files.",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                    "default": None,
                    "metadata": {"tags": ["byo", "bios"]},
                }
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(AddInputCommand(input_id="new_input")))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "label", "New Input")))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "type", "directory")))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "role", "bios")))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "description", "New description")))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "required", True)))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "multiple", True)))
            self.assertTrue(document.apply_command(UpdateInputFieldCommand("new_input", "validation.must_exist", True)))
            self.assertTrue(
                document.apply_command(
                    UpdateInputFieldCommand("new_input", "validation.allowed_extensions", "zip, cfg")
                )
            )
            self.assertTrue(
                document.apply_command(UpdateInputFieldCommand("new_input", "validation.path_kind", "directory"))
            )
            self.assertTrue(
                document.apply_command(
                    DuplicateInputCommand(source_input_id="bios_source_dir", new_input_id="bios_source_dir_copy")
                )
            )
            self.assertTrue(document.apply_command(DeleteInputCommand(input_id="new_input")))

            original = document.working_recipe.inputs["bios_source_dir"]
            duplicate = document.working_recipe.inputs["bios_source_dir_copy"]
            self.assertEqual(original.metadata, {"tags": ["byo", "bios"]})
            self.assertEqual(duplicate.metadata, {"tags": ["byo", "bios"]})
            self.assertEqual(duplicate.validation.path_kind.value, "directory")
            self.assertIn("metadata:", document.to_yaml())
            self.assertIn("bios_source_dir_copy:", document.to_yaml())

    def test_artifact_commands_and_group_membership_cascade(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "first_zip": {"type": "remote_file", "url": "https://example.com/first.zip"},
                "second_zip": {"type": "remote_file", "url": "https://example.com/second.zip"},
            },
            artifact_groups={"bundle": ["first_zip", "second_zip"]},
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(AddArtifactCommand(artifact_id="third_zip", url="https://example.com/third.zip")))
            self.assertTrue(
                document.apply_command(
                    UpdateArtifactFieldCommand("third_zip", "cache", "none")
                )
            )
            self.assertTrue(
                document.apply_command(
                    DuplicateArtifactCommand(source_artifact_id="first_zip", new_artifact_id="first_zip_copy")
                )
            )
            self.assertTrue(document.apply_command(DeleteArtifactCommand(artifact_id="second_zip")))

            self.assertEqual(tuple(document.working_recipe.artifacts), ("first_zip", "third_zip", "first_zip_copy"))
            self.assertEqual(document.working_recipe.artifacts["third_zip"].cache.value, "none")
            self.assertEqual(document.working_recipe.artifact_groups["bundle"], ("first_zip",))

    def test_artifact_group_commands_preserve_group_and_membership_order(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "alpha_zip": {"type": "remote_file", "url": "https://example.com/a.zip"},
                "beta_zip": {"type": "remote_file", "url": "https://example.com/b.zip"},
                "gamma_zip": {"type": "remote_file", "url": "https://example.com/c.zip"},
            },
            artifact_groups={
                "first_group": ["alpha_zip"],
                "second_group": ["beta_zip", "gamma_zip"],
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(AddArtifactGroupCommand(group_id="third_group")))
            self.assertTrue(document.apply_command(ReorderArtifactGroupCommand(group_id="third_group", to_index=0)))
            self.assertTrue(
                document.apply_command(
                    AddArtifactGroupMemberCommand(group_id="third_group", artifact_id="gamma_zip", index=0)
                )
            )
            self.assertTrue(
                document.apply_command(
                    AddArtifactGroupMemberCommand(group_id="third_group", artifact_id="alpha_zip", index=1)
                )
            )
            self.assertTrue(
                document.apply_command(
                    ReorderArtifactGroupMemberCommand(group_id="third_group", index=1, to_index=0)
                )
            )
            self.assertTrue(
                document.apply_command(RemoveArtifactGroupMemberCommand(group_id="third_group", index=1))
            )

            self.assertEqual(tuple(document.working_recipe.artifact_groups), ("third_group", "first_group", "second_group"))
            self.assertEqual(document.working_recipe.artifact_groups["third_group"], ("alpha_zip",))

    def test_dirty_history_and_save_undo_redo_follow_saved_baseline(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            document = load_recipe_document(recipe_path)

            self.assertFalse(document.can_undo)
            self.assertFalse(document.can_redo)
            self.assertFalse(document.apply_command(SetOverviewFieldCommand(field="name", value="Example Recipe")))
            self.assertFalse(document.is_dirty)

            self.assertTrue(document.apply_command(SetOverviewFieldCommand(field="name", value="Renamed Recipe")))
            self.assertTrue(document.is_dirty)
            self.assertTrue(document.can_undo)
            self.assertFalse(document.can_redo)

            document.save()
            self.assertFalse(document.is_dirty)
            self.assertTrue(document.can_undo)

            self.assertTrue(document.undo())
            self.assertTrue(document.is_dirty)
            self.assertTrue(document.can_redo)
            self.assertEqual(document.working_recipe.name, "Example Recipe")

            self.assertTrue(document.redo())
            self.assertFalse(document.is_dirty)
            self.assertEqual(document.working_recipe.name, "Renamed Recipe")

    def test_catalog_context_validation_replaces_open_document_by_path(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        sibling = base_recipe(recipe_id="sibling.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe, sibling])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml", authored_root=authored_root)

            self.assertTrue(document.apply_command(SetOverviewFieldCommand(field="id", value="sibling.recipe")))

            conflict_codes = [item.code for item in document.validation_result.diagnostics]
            self.assertIn("recipe_id_conflict", conflict_codes)

    def test_yaml_writer_preserves_input_artifact_group_and_membership_order(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "zeta_input": {
                    "type": "file",
                    "role": "generic",
                    "label": "Zeta",
                    "required": False,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": [], "path_kind": "file"},
                },
                "alpha_input": {
                    "type": "file",
                    "role": "generic",
                    "label": "Alpha",
                    "required": False,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": [], "path_kind": "file"},
                },
            },
            artifacts={
                "zeta_artifact": {"type": "remote_file", "url": "https://example.com/z.zip"},
                "alpha_artifact": {"type": "remote_file", "url": "https://example.com/a.zip"},
            },
            artifact_groups={
                "later_group": ["zeta_artifact", "alpha_artifact"],
                "earlier_group": ["alpha_artifact"],
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(AddInputCommand(input_id="middle_input")))
            self.assertTrue(document.apply_command(AddArtifactCommand(artifact_id="middle_artifact", url="https://example.com/m.zip")))
            self.assertTrue(document.apply_command(AddArtifactGroupCommand(group_id="final_group")))
            self.assertTrue(
                document.apply_command(
                    AddArtifactGroupMemberCommand(group_id="final_group", artifact_id="middle_artifact", index=0)
                )
            )

            emitted = document.to_yaml()
            self.assertLess(emitted.index("  zeta_input:"), emitted.index("  alpha_input:"))
            self.assertLess(emitted.index("  alpha_input:"), emitted.index("  middle_input:"))
            self.assertLess(emitted.index("  zeta_artifact:"), emitted.index("  alpha_artifact:"))
            self.assertLess(emitted.index("  alpha_artifact:"), emitted.index("  middle_artifact:"))
            self.assertLess(emitted.index("  later_group:"), emitted.index("  earlier_group:"))
            self.assertLess(emitted.index("  earlier_group:"), emitted.index("  final_group:"))
            self.assertIn("  later_group:\n  - zeta_artifact\n  - alpha_artifact", emitted)
            self.assertIn("  final_group:\n  - middle_artifact", emitted)


if __name__ == "__main__":
    unittest.main()
