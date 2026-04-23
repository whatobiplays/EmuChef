from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import yaml

from emuchef.domain import (
    AppOpGrant,
    PermissionWhen,
    RefParamValue,
    RuntimePermissionGrant,
    StepCondition,
    StepConstraints,
    StepType,
)
from emuchef.io import load_authored_recipe
from emuchef_editor.app.workspace.service import open_workspace, resolve_authored_root
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddAppOpCommand,
    AddInputCommand,
    AddProvidedFeatureCommand,
    AddRecipeDependencyCommand,
    AddRuntimePermissionCommand,
    AddStepCommand,
    DeleteArtifactCommand,
    DeleteAppOpCommand,
    DeleteInputCommand,
    DeleteRuntimePermissionCommand,
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
    SetStepUserToggleableCommand,
    SetOverviewFieldCommand,
    UpdateStepBasicsCommand,
    UpdateStepConstraintsCommand,
    UpdateStepDependenciesCommand,
    UpdateStepParamsCommand,
    UpdateStepSkipIfCommand,
    UpdateStepVerifyCommand,
    UpdateAppOpCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdatePermissionPolicyFieldCommand,
    UpdateProvidedFeatureCommand,
    UpdateRecipeDependencyCommand,
    UpdateRuntimePermissionCommand,
)
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

    def test_permission_commands_support_runtime_appops_policy_and_when_normalization(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(
                document.apply_command(
                    AddRuntimePermissionCommand(
                        RuntimePermissionGrant(
                            package_name="com.example.app",
                            name="READ_MEDIA_VIDEO",
                        )
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateRuntimePermissionCommand(
                        index=0,
                        permission=RuntimePermissionGrant(
                            package_name="com.example.updated",
                            name="READ_MEDIA_AUDIO",
                            required=False,
                            when=PermissionWhen(),
                        ),
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    AddAppOpCommand(
                        AppOpGrant(
                            package_name="com.example.updated",
                            op="WRITE_SETTINGS",
                            mode="allow",
                        )
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdateAppOpCommand(
                        index=0,
                        appop=AppOpGrant(
                            package_name="com.example.updated",
                            op="SYSTEM_ALERT_WINDOW",
                            mode="ignore",
                            required=False,
                            when=PermissionWhen(rooted=True, android_api_min=30, android_api_max=34),
                        ),
                    )
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdatePermissionPolicyFieldCommand(field="on_failure", value="fail")
                )
            )
            self.assertTrue(
                document.apply_command(
                    UpdatePermissionPolicyFieldCommand(field="require_all", value=True)
                )
            )
            self.assertTrue(document.apply_command(DeleteRuntimePermissionCommand(index=0)))
            self.assertTrue(
                document.apply_command(
                    AddRuntimePermissionCommand(
                        RuntimePermissionGrant(
                            package_name="com.example.updated",
                            name="POST_NOTIFICATIONS",
                            required=False,
                            when=PermissionWhen(rooted=False, android_api_min=33),
                        )
                    )
                )
            )
            self.assertTrue(document.apply_command(DeleteAppOpCommand(index=0)))

            permissions = document.working_recipe.permissions
            self.assertEqual(len(permissions.runtime), 1)
            self.assertEqual(permissions.runtime[0].package_name, "com.example.updated")
            self.assertEqual(permissions.runtime[0].when.rooted, False)
            self.assertEqual(permissions.runtime[0].when.android_api_min, 33)
            self.assertIsNone(permissions.runtime[0].when.android_api_max)
            self.assertEqual(permissions.appops, ())
            self.assertEqual(permissions.policy.on_failure, "fail")
            self.assertTrue(permissions.policy.require_all)
            self.assertIn("permissions:", document.to_yaml())
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
            steps=[],
            permissions={
                "runtime": [{"package_name": "com.example.app", "name": "POST_NOTIFICATIONS"}],
                "policy": {"on_failure": "warn", "require_all": False},
            },
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
                ("resolve", StepType.RESOLVE_ARTIFACTS, "Resolve Artifacts"),
                ("extract", StepType.EXTRACT_ARTIFACTS, "Extract Artifacts"),
                ("unpack", StepType.EXTRACT_ARCHIVE, "Extract Archive"),
                ("copy", StepType.COPY_FILES, "Copy Files"),
                ("install", StepType.INSTALL_APK, "Install APK"),
                ("grant", StepType.GRANT_PERMISSIONS, "Grant Permissions"),
                ("launch", StepType.LAUNCH_APP, "Launch App"),
                ("pause", StepType.WAIT, "Wait"),
                ("stop", StepType.FORCE_STOP_APP, "Force Stop"),
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
                    StepType.COPY_FILES,
                    StepType.RESOLVE_ARTIFACTS,
                    StepType.EXTRACT_ARTIFACTS,
                    StepType.EXTRACT_ARCHIVE,
                    StepType.COPY_FILES,
                    StepType.INSTALL_APK,
                    StepType.LAUNCH_APP,
                    StepType.WAIT,
                    StepType.FORCE_STOP_APP,
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

    def test_step_delete_preserves_broken_dependency_for_diagnostics(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "prepare",
                    "type": "wait",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 1000},
                    "verify": [],
                },
                {
                    "id": "consume",
                    "type": "wait",
                    "name": "Consume",
                    "user_toggleable": False,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 1000},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml")

            self.assertTrue(document.apply_command(DeleteStepCommand(step_id="prepare")))

            self.assertEqual(tuple(step.id for step in document.working_recipe.steps), ("consume",))
            self.assertEqual(document.working_recipe.steps[0].dependencies, ("prepare",))
            diagnostics = document.validation_result.diagnostics
            self.assertIn("step_not_found", tuple(item.code for item in diagnostics))
            dependency_error = next(item for item in diagnostics if item.code == "step_not_found")
            self.assertEqual(dependency_error.field, "steps[0].dependencies[0]")

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
