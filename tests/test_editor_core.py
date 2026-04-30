from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import yaml

from emuchef.domain import (
    RefParamValue,
    StepCondition,
    StepConstraints,
)
from emuchef.io import load_authored_recipe
from emuchef_editor.app.workspace.service import open_workspace, resolve_authored_root
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddInputCommand,
    AddProvidedFeatureCommand,
    AddRecipeDependencyCommand,
    AddStepCommand,
    DeleteArtifactCommand,
    DeleteArtifactGroupCommand,
    DeleteInputCommand,
    DeleteStepCommand,
    DuplicateArtifactCommand,
    DuplicateInputCommand,
    DuplicateStepCommand,
    MoveProvidedFeatureCommand,
    MoveRecipeDependencyCommand,
    RemoveArtifactGroupMemberCommand,
    RemoveProvidedFeatureCommand,
    RemoveRecipeDependencyCommand,
    ReorderArtifactGroupCommand,
    ReorderArtifactGroupMemberCommand,
    ReorderStepCommand,
    RenameArtifactCommand,
    RenameArtifactGroupCommand,
    RenameInputCommand,
    RenameRecipeIdCommand,
    RenameStepCommand,
    SetStepUserToggleableCommand,
    SetOverviewFieldCommand,
    UpdateStepBasicsCommand,
    UpdateStepConstraintsCommand,
    UpdateStepDependenciesCommand,
    UpdateStepParamsCommand,
    UpdateStepSkipIfCommand,
    UpdateStepVerifyCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdateProvidedFeatureCommand,
    UpdateRecipeDependencyCommand,
)
from emuchef_editor.core.analysis.usages import UsageTarget, analyze_recipe_usages
from emuchef_editor.core.validation.validator_service import ValidatorService
from emuchef_editor.core.yaml.loader import create_recipe_document_from_template, load_recipe_document

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
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
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
            candidate_by_ref = {candidate.ref: candidate for candidate in document.ref_index.candidates}
            self.assertEqual(candidate_by_ref["inputs.source_dir"].value_type.value, "directory_path")
            self.assertEqual(candidate_by_ref["artifacts.archive_zip.filename"].value_type.value, "string")
            self.assertEqual(
                candidate_by_ref["steps.extract_archive.outputs.extracted_path"].value_type.value,
                "directory_path",
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

    def test_workspace_discovery_lists_recipe_templates_separately(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        blank_template = base_recipe(recipe_id="blank.recipe", name="Blank Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={
                    "recipe.template.yaml": template,
                    "recipe.blank.template.yaml": blank_template,
                    "device_plan.template.yaml": {
                        "schema_version": 1,
                        "kind": "device_plan",
                        "id": "not.a.recipe",
                        "name": "Not A Recipe",
                        "description": "Should be ignored by recipe template discovery.",
                        "device_profile_ref": "example.device_profile",
                        "recipes": [],
                        "defaults": {},
                        "overrides": {},
                        "metadata": {},
                    },
                },
            )

            repo_workspace = open_workspace(repo_root)
            authored_workspace = open_workspace(authored_root)

            self.assertEqual(
                tuple(path.name for path in repo_workspace.template_files),
                ("recipe.blank.template.yaml", "recipe.template.yaml"),
            )
            self.assertEqual(repo_workspace.template_files, authored_workspace.template_files)

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

    def test_delete_artifact_cleans_group_membership_step_selection_and_param_ref_in_one_command(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "target_zip": {"type": "remote_file", "url": "https://example.com/target.zip"},
                "other_zip": {"type": "remote_file", "url": "https://example.com/other.zip"},
            },
            artifact_groups={"bundle": ["target_zip", "other_zip"]},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["target_zip", "other_zip"]},
                    "verify": [],
                },
                {
                    "id": "extract",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "artifacts.target_zip.local_path"}},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(DeleteArtifactCommand(artifact_id="target_zip")))

            self.assertNotIn("target_zip", document.working_recipe.artifacts)
            self.assertEqual(document.working_recipe.artifact_groups["bundle"], ("other_zip",))
            resolve_step = next(step for step in document.working_recipe.steps if step.id == "resolve")
            extract_step = next(step for step in document.working_recipe.steps if step.id == "extract")
            self.assertEqual(resolve_step.params["artifacts"], ["other_zip"])
            self.assertNotIn("archive", extract_step.params)
            emitted = document.to_yaml()
            self.assertNotIn("target_zip:", emitted)
            self.assertNotIn("target_zip\n", emitted)
            self.assertNotIn("artifacts.target_zip.local_path", emitted)

            self.assertTrue(document.undo())
            self.assertIn("target_zip", document.working_recipe.artifacts)
            self.assertEqual(document.working_recipe.artifact_groups["bundle"], ("target_zip", "other_zip"))
            self.assertEqual(
                next(step for step in document.working_recipe.steps if step.id == "extract").params["archive"].ref,
                "artifacts.target_zip.local_path",
            )

    def test_delete_input_and_artifact_group_remove_supported_structured_refs_only(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            artifacts={
                "base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"},
            },
            artifact_groups={"bundle": ["base_zip"], "other_bundle": ["base_zip"]},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifact_groups": ["bundle", "other_bundle"]},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(DeleteInputCommand(input_id="source_dir")))
            self.assertTrue(document.apply_command(DeleteArtifactGroupCommand(group_id="bundle")))

            copy_step = next(step for step in document.working_recipe.steps if step.id == "copy")
            resolve_step = next(step for step in document.working_recipe.steps if step.id == "resolve")
            self.assertNotIn("source", copy_step.params)
            self.assertEqual(copy_step.params["dest"], "/sdcard/Example")
            self.assertNotIn("bundle", document.working_recipe.artifact_groups)
            self.assertEqual(resolve_step.params["artifact_groups"], ["other_bundle"])

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

    def test_rename_commands_rewrite_supported_structured_usages(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            recipe_dependencies=["example.recipe"],
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            artifacts={
                "archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"},
                "base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"},
            },
            artifact_groups={"bundle": ["archive_zip", "base_zip"]},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["archive_zip"], "artifact_groups": ["bundle"]},
                    "verify": [],
                },
                {
                    "id": "extract",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "artifacts.archive_zip.local_path"}},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": ["extract"],
                    "constraints": {"capabilities": [], "conflicts_with": ["extract"]},
                    "params": {"source": {"ref": "steps.extract.outputs.extracted_path"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
                {
                    "id": "copy_input",
                    "type": "copy_files",
                    "name": "Copy Input",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Input"},
                    "verify": [],
                },
                {
                    "id": "copy_shorthand",
                    "type": "copy_files",
                    "name": "Copy Shorthand",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "steps.extract"}, "dest": "/sdcard/Shorthand"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(RenameRecipeIdCommand(new_recipe_id="updated.recipe")))
            self.assertTrue(document.apply_command(RenameInputCommand(input_id="source_dir", new_input_id="renamed_source")))
            self.assertTrue(document.apply_command(RenameArtifactCommand(artifact_id="archive_zip", new_artifact_id="renamed_archive")))
            self.assertTrue(document.apply_command(RenameArtifactGroupCommand(group_id="bundle", new_group_id="renamed_bundle")))
            self.assertTrue(document.apply_command(RenameStepCommand(step_id="extract", new_step_id="unpack")))

            self.assertEqual(document.working_recipe.id, "updated.recipe")
            self.assertEqual(document.working_recipe.recipe_dependencies, ("updated.recipe",))
            self.assertIn("renamed_source", document.working_recipe.inputs)
            self.assertIn("renamed_archive", document.working_recipe.artifacts)
            self.assertEqual(document.working_recipe.artifact_groups["renamed_bundle"], ("renamed_archive", "base_zip"))
            resolve_step = next(step for step in document.working_recipe.steps if step.id == "resolve")
            copy_step = next(step for step in document.working_recipe.steps if step.id == "copy")
            copy_input_step = next(step for step in document.working_recipe.steps if step.id == "copy_input")
            copy_shorthand_step = next(step for step in document.working_recipe.steps if step.id == "copy_shorthand")
            self.assertEqual(resolve_step.params["artifacts"], ["renamed_archive"])
            self.assertEqual(resolve_step.params["artifact_groups"], ["renamed_bundle"])
            self.assertEqual(copy_step.dependencies, ("unpack",))
            self.assertEqual(copy_step.constraints.conflicts_with, ("unpack",))
            self.assertEqual(copy_step.params["source"].ref, "steps.unpack.outputs.extracted_path")
            self.assertEqual(copy_input_step.params["source"].ref, "inputs.renamed_source")
            self.assertEqual(copy_shorthand_step.params["source"].ref, "steps.unpack")
            self.assertEqual(
                next(step for step in document.working_recipe.steps if step.id == "unpack").params["archive"].ref,
                "artifacts.renamed_archive.local_path",
            )

            emitted = document.to_yaml()
            self.assertIn("id: updated.recipe", emitted)
            self.assertIn("ref: inputs.renamed_source", emitted)
            self.assertIn("ref: artifacts.renamed_archive.local_path", emitted)
            self.assertIn("ref: steps.unpack.outputs.extracted_path", emitted)

    def test_grant_permission_params_are_normal_step_params(self) -> None:
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

            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="grant",
                        params={
                            "runtime": [
                                {
                                    "package_name": "com.example.updated",
                                    "name": "POST_NOTIFICATIONS",
                                    "required": False,
                                    "when": {"rooted": False, "android_api_min": 33},
                                }
                            ],
                            "appops": [],
                            "policy": {"on_failure": "fail", "require_all": True},
                        },
                    )
                )
            )

            grant = document.working_recipe.steps[0]
            self.assertEqual(grant.params["runtime"][0]["package_name"], "com.example.updated")
            self.assertEqual(grant.params["runtime"][0]["when"]["rooted"], False)
            self.assertEqual(grant.params["runtime"][0]["when"]["android_api_min"], 33)
            self.assertEqual(grant.params["appops"], [])
            self.assertEqual(grant.params["policy"]["on_failure"], "fail")
            self.assertTrue(grant.params["policy"]["require_all"])
            self.assertNotIn("permissions:", document.to_yaml())
            self.assertIn("POST_NOTIFICATIONS", document.to_yaml())
            self.assertIn("on_failure: fail", document.to_yaml())
            self.assertIn("require_all: true", document.to_yaml())

    def test_save_as_updates_document_path_and_preserves_history(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            original_path = authored_root / "recipes" / "example_recipe.yaml"
            save_as_path = authored_root / "recipes" / "copied_recipe.yaml"
            document = load_recipe_document(original_path)

            self.assertTrue(
                document.apply_command(SetOverviewFieldCommand(field="name", value="Renamed Recipe"))
            )
            self.assertTrue(document.can_undo)
            payload = document.save_as(save_as_path)

            self.assertEqual(document.path, save_as_path.resolve())
            self.assertTrue(save_as_path.exists())
            self.assertEqual(save_as_path.read_text(encoding="utf-8"), payload)
            self.assertFalse(document.is_dirty)
            self.assertTrue(document.can_undo)

            self.assertTrue(document.undo())
            self.assertTrue(document.is_dirty)
            self.assertEqual(document.working_recipe.name, "Example Recipe")

            self.assertTrue(document.redo())
            self.assertFalse(document.is_dirty)
            self.assertEqual(document.working_recipe.name, "Renamed Recipe")

    def test_create_recipe_document_from_template_writes_destination_and_opens_clean_document(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(
            recipe_id="template.recipe",
            name="Template Recipe",
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "runtime": [{"package_name": "com.example.app", "name": "POST_NOTIFICATIONS"}],
                        "policy": {"on_failure": "warn", "require_all": False},
                    },
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={"recipe.template.yaml": template},
            )
            destination_path = authored_root / "recipes" / "new_recipe.yaml"

            document = create_recipe_document_from_template(
                repo_root / "templates" / "authored" / "recipe.template.yaml",
                destination_path=destination_path,
                recipe_id="created.recipe",
                authored_root=authored_root,
            )

            self.assertEqual(document.path, destination_path.resolve())
            self.assertEqual(document.working_recipe.id, "created.recipe")
            self.assertEqual(document.working_recipe.name, "Template Recipe")
            self.assertFalse(document.is_dirty)
            self.assertFalse(document.can_undo)
            self.assertTrue(destination_path.exists())
            written = destination_path.read_text(encoding="utf-8")
            self.assertIn("id: created.recipe", written)
            self.assertIn("name: Template Recipe", written)

    def test_step_commands_cover_supported_types_and_default_param_omission(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            artifacts={
                "archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"},
                "app_apk": {"type": "remote_file", "url": "https://example.com/app.apk"},
                "base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"},
            },
            artifact_groups={"bundle": ["base_zip"]},
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            additions = (
                ("resolve", "resolve_artifacts", "Resolve Artifacts"),
                ("extract", "extract_artifacts", "Extract Artifacts"),
                ("unpack", "extract_archive", "Extract Archive"),
                ("copy", "copy_files", "Copy Files"),
                ("install", "install_apk", "Install APK"),
                ("grant", "grant_permissions", "Grant Permissions"),
                ("launch", "launch_app", "Launch App"),
                ("pause", "wait", "Wait"),
                ("stop", "force_stop_app", "Force Stop"),
            )
            for step_id, step_type, name in additions:
                self.assertTrue(document.apply_command(AddStepCommand(step_id=step_id, step_type=step_type, name=name)))

            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="resolve",
                        params={"artifacts": ["base_zip"], "artifact_groups": ["bundle"]},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="extract",
                        params={"artifacts": ["base_zip"], "extract_on": "device"},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="extract",
                        params={"artifacts": ["base_zip"], "extract_on": "host"},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="unpack",
                        params={
                            "archive": RefParamValue(ref="artifacts.archive_zip.local_path"),
                            "extract_on": "device",
                            "dest": "/sdcard/Extracted",
                            "device_temp_path": "/data/local/tmp/archive",
                            "cleanup": False,
                        },
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="unpack",
                        params={"archive": RefParamValue(ref="artifacts.archive_zip.local_path"), "extract_on": "host", "cleanup": True},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="copy",
                        params={
                            "source": RefParamValue(ref="inputs.source_dir"),
                            "dest": "/sdcard/RetroArch/cores",
                            "copy_policy": "sync",
                        },
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="install",
                        params={"app": RefParamValue(ref="artifacts.app_apk.local_path"), "replace_existing": False},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="launch",
                        params={"package_name": "com.example.app", "activity": "MainActivity"},
                    )
                )
            )
            self.assertTrue(
                document.apply_command(UpdateStepParamsCommand(step_id="pause", params={"duration_ms": 1500}))
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(step_id="stop", params={"package_name": "com.example.app"})
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepBasicsCommand(step_id="copy", name="Copy Assets", description="Copy staged files.")
                )
            )
            self.assertTrue(document.apply_command(SetStepUserToggleableCommand(step_id="copy", user_toggleable=True)))
            self.assertTrue(
                document.apply_command(
                    UpdateStepDependenciesCommand(step_id="copy", dependencies=("resolve", "extract"))
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepConstraintsCommand(
                        step_id="copy",
                        constraints=StepConstraints(
                            capabilities=("shared_storage_write",),
                            conflicts_with=("stop",),
                        ),
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepSkipIfCommand(
                        step_id="copy",
                        skip_if=(StepCondition(type="package_installed", params={"package_name": "com.example.app"}),),
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateStepVerifyCommand(
                        step_id="copy",
                        verify=(StepCondition(type="path_exists", params={"path": "/sdcard/RetroArch/cores"}),),
                    )
                )
            )
            self.assertTrue(
                document.apply_command(DuplicateStepCommand(source_step_id="copy", new_step_id="copy_duplicate"))
            )
            self.assertTrue(document.apply_command(ReorderStepCommand(step_id="copy_duplicate", to_index=0)))
            self.assertTrue(document.apply_command(DeleteStepCommand(step_id="grant")))

            self.assertEqual(document.working_recipe.steps[0].id, "copy_duplicate")
            self.assertEqual(
                tuple(step.type for step in document.working_recipe.steps),
                (
                    "copy_files",
                    "resolve_artifacts",
                    "extract_artifacts",
                    "extract_archive",
                    "copy_files",
                    "install_apk",
                    "launch_app",
                    "wait",
                    "force_stop_app",
                ),
            )
            copy_step = next(step for step in document.working_recipe.steps if step.id == "copy")
            self.assertTrue(copy_step.user_toggleable)
            self.assertEqual(copy_step.dependencies, ("resolve", "extract"))
            self.assertEqual(copy_step.constraints.capabilities, ("shared_storage_write",))
            self.assertEqual(copy_step.constraints.conflicts_with, ("stop",))
            self.assertEqual(copy_step.skip_if[0].type, "package_installed")
            self.assertEqual(copy_step.verify[0].type, "path_exists")
            self.assertEqual(copy_step.params["copy_policy"], "sync")
            self.assertEqual(copy_step.params["source"].ref, "inputs.source_dir")

            install_step = next(step for step in document.working_recipe.steps if step.id == "install")
            unpack_step = next(step for step in document.working_recipe.steps if step.id == "unpack")
            extract_step = next(step for step in document.working_recipe.steps if step.id == "extract")
            self.assertNotIn("replace_existing", install_step.params)
            self.assertEqual(tuple(unpack_step.params), ("archive",))
            self.assertEqual(extract_step.params, {"artifacts": ["base_zip"]})

            emitted = document.to_yaml()
            self.assertIn("steps:", emitted)
            self.assertIn("ref: artifacts.archive_zip.local_path", emitted)
            self.assertIn("ref: inputs.source_dir", emitted)
            self.assertNotIn("replace_existing: false", emitted)
            self.assertNotIn("extract_on: host", emitted)
            self.assertNotIn("cleanup: true", emitted)

    def test_step_delete_removes_supported_dependencies_conflicts_and_refs(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "prepare",
                    "type": "extract_archive",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "steps.seed.outputs.copied_paths"}},
                    "verify": [],
                },
                {
                    "id": "consume",
                    "type": "copy_files",
                    "name": "Consume",
                    "user_toggleable": False,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": [], "conflicts_with": ["prepare"]},
                    "params": {"source": {"ref": "steps.prepare.outputs.extracted_path"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
                {
                    "id": "consume_shorthand",
                    "type": "copy_files",
                    "name": "Consume Shorthand",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "steps.prepare"}, "dest": "/sdcard/Example2"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(DeleteStepCommand(step_id="prepare")))

            self.assertEqual(tuple(step.id for step in document.working_recipe.steps), ("consume", "consume_shorthand"))
            consume_step = document.working_recipe.steps[0]
            shorthand_step = document.working_recipe.steps[1]
            self.assertEqual(consume_step.dependencies, ())
            self.assertEqual(consume_step.constraints.conflicts_with, ())
            self.assertNotIn("source", consume_step.params)
            self.assertNotIn("source", shorthand_step.params)
            self.assertNotIn("steps.prepare", document.to_yaml())

    def test_supported_step_edit_preserves_serialized_unsupported_content(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            steps=[
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {
                        "capabilities": ["shared_storage_write", "unsupported_capability"],
                        "conflicts_with": ["missing_conflict"],
                    },
                    "params": {
                        "source": {"ref": "inputs.source_dir"},
                        "dest": "/sdcard/Example",
                        "experimental_mode": {"enabled": True},
                    },
                    "skip_if": [
                        {"type": "package_installed", "params": {"package_name": "com.example.app"}},
                        {"type": "custom_skip", "params": {"foo": "bar"}},
                    ],
                    "verify": [
                        {"type": "path_exists", "params": {"path": "/sdcard/Example"}},
                        {"type": "custom_verify", "params": {"bar": "baz"}},
                    ],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")
            before_yaml = document.to_yaml()
            before_payload = yaml.safe_load(before_yaml)

            self.assertTrue(
                document.apply_command(
                    UpdateStepBasicsCommand(
                        step_id="copy",
                        name="Copy Updated",
                        description="Updated description.",
                    )
                )
            )

            after_yaml = document.to_yaml()
            after_payload = yaml.safe_load(after_yaml)
            before_step = before_payload["steps"][0]
            after_step = after_payload["steps"][0]
            self.assertEqual(before_step["params"]["experimental_mode"], after_step["params"]["experimental_mode"])
            self.assertEqual(before_step["skip_if"][1], after_step["skip_if"][1])
            self.assertEqual(before_step["verify"][1], after_step["verify"][1])
            self.assertEqual(before_step["constraints"], after_step["constraints"])
            self.assertIn("experimental_mode:", after_yaml)
            self.assertIn("custom_skip", after_yaml)
            self.assertIn("custom_verify", after_yaml)
            self.assertIn("unsupported_capability", after_yaml)
            self.assertIn("missing_conflict", after_yaml)

    def test_rename_preserves_unsupported_step_content_semantically(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            steps=[
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "source": {"ref": "inputs.source_dir"},
                        "dest": "/sdcard/Example",
                        "experimental_source": {"ref": "inputs.source_dir"},
                    },
                    "skip_if": [
                        {"type": "custom_skip", "params": {"target": "inputs.source_dir"}},
                    ],
                    "verify": [
                        {"type": "custom_verify", "params": {"target": "inputs.source_dir"}},
                    ],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")
            before_step = yaml.safe_load(document.to_yaml())["steps"][0]

            self.assertTrue(document.apply_command(RenameInputCommand(input_id="source_dir", new_input_id="renamed_source")))

            after_step = yaml.safe_load(document.to_yaml())["steps"][0]
            self.assertEqual(after_step["params"]["source"]["ref"], "inputs.renamed_source")
            self.assertEqual(after_step["params"]["experimental_source"], before_step["params"]["experimental_source"])
            self.assertEqual(after_step["skip_if"], before_step["skip_if"])
            self.assertEqual(after_step["verify"], before_step["verify"])
            self.assertIn("renamed_source", document.working_recipe.inputs)

    def test_usage_analysis_groups_supported_usages_and_flags_preserved_content(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"},
            },
            artifact_groups={"bundle": ["archive_zip"]},
            steps=[
                {
                    "id": "extract",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "artifacts.archive_zip.local_path"}},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": ["extract"],
                    "constraints": {"capabilities": ["unsupported_capability"], "conflicts_with": ["extract"]},
                    "params": {
                        "source": {"ref": "steps.extract.outputs.extracted_path"},
                        "dest": "/sdcard/Example",
                        "experimental_ref": {"ref": "steps.extract"},
                    },
                    "skip_if": [{"type": "custom_skip", "params": {"foo": "bar"}}],
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            step_analysis = analyze_recipe_usages(
                document.working_recipe,
                UsageTarget(kind="step", id="extract"),
            )
            grouped = {group.title: tuple(usage.summary for usage in group.usages) for group in step_analysis.groups}

            self.assertIn("Param Refs", grouped)
            self.assertIn("Dependencies", grouped)
            self.assertIn("Constraints / Conflicts", grouped)
            self.assertTrue(any("copy" in summary and "source" in summary for summary in grouped["Param Refs"]))
            self.assertTrue(any("copy" in summary for summary in grouped["Dependencies"]))
            self.assertTrue(any("copy" in summary for summary in grouped["Constraints / Conflicts"]))
            self.assertTrue(step_analysis.has_preserved_unsupported_content_warning)

            no_usage_analysis = analyze_recipe_usages(
                document.working_recipe,
                UsageTarget(kind="input", id="missing_input"),
            )
            self.assertEqual(no_usage_analysis.groups, ())

    def test_step_output_refs_remain_explicit_and_do_not_add_dependencies(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"},
            },
            steps=[
                {
                    "id": "extract",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "artifacts.archive_zip.local_path"}},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"dest": "/sdcard/Example"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(
                document.apply_command(
                    UpdateStepParamsCommand(
                        step_id="copy",
                        params={
                            "source": RefParamValue(ref="steps.extract.outputs.extracted_path"),
                            "dest": "/sdcard/Example",
                        },
                    )
                )
            )

            copy_step = next(step for step in document.working_recipe.steps if step.id == "copy")
            self.assertEqual(copy_step.dependencies, ())
            self.assertEqual(copy_step.params["source"].ref, "steps.extract.outputs.extracted_path")
            self.assertIn("ref: steps.extract.outputs.extracted_path", document.to_yaml())
            self.assertNotIn("ref: steps.extract\n", document.to_yaml())

    def test_editing_supported_step_fields_preserves_unresolved_refs(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "source": {"ref": "steps.missing.outputs.copied_paths"},
                        "dest": "/sdcard/Example",
                    },
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(
                document.apply_command(
                    UpdateStepBasicsCommand(
                        step_id="copy",
                        name="Copy Updated",
                        description="Updated description.",
                    )
                )
            )

            copy_step = document.working_recipe.steps[0]
            self.assertEqual(copy_step.params["source"].ref, "steps.missing.outputs.copied_paths")
            self.assertIn("ref: steps.missing.outputs.copied_paths", document.to_yaml())


if __name__ == "__main__":
    unittest.main()
