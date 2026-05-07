from __future__ import annotations

import unittest

from emuchef.domain import RefParamValue
from emuchef_editor.api.command_codec import decode_recipe_command
from emuchef_editor.api.errors import ApiError
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddInputCommand,
    AddStepCommand,
    DeleteArtifactCommand,
    DeleteArtifactGroupCommand,
    DeleteInputCommand,
    DeleteStepCommand,
    DuplicateArtifactCommand,
    DuplicateArtifactGroupCommand,
    DuplicateInputCommand,
    DuplicateStepCommand,
    RemoveArtifactGroupMemberCommand,
    RenameArtifactCommand,
    RenameArtifactGroupCommand,
    RenameInputCommand,
    ReorderArtifactGroupCommand,
    ReorderArtifactGroupMemberCommand,
    ReorderStepCommand,
    SetOverviewFieldCommand,
    SetStepUserToggleableCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdateStepBasicsCommand,
    UpdateStepDependenciesCommand,
    UpdateStepParamsCommand,
)


class EditorApiCommandCodecTests(unittest.TestCase):
    def test_required_phase_one_command_mappings_decode_explicitly(self) -> None:
        cases = [
            (
                {"type": "SetOverviewField", "field": "name", "value": "Updated"},
                SetOverviewFieldCommand(field="name", value="Updated"),
            ),
            ({"type": "AddInput", "inputId": "roms_dir"}, AddInputCommand(input_id="roms_dir")),
            (
                {"type": "RenameInput", "inputId": "roms_dir", "newInputId": "games_dir"},
                RenameInputCommand(input_id="roms_dir", new_input_id="games_dir"),
            ),
            ({"type": "DeleteInput", "inputId": "roms_dir"}, DeleteInputCommand(input_id="roms_dir")),
            (
                {"type": "DuplicateInput", "sourceInputId": "roms_dir", "newInputId": "games_dir"},
                DuplicateInputCommand(source_input_id="roms_dir", new_input_id="games_dir"),
            ),
            (
                {"type": "UpdateInputField", "inputId": "roms_dir", "field": "label", "value": "ROMs"},
                UpdateInputFieldCommand(input_id="roms_dir", field="label", value="ROMs"),
            ),
            (
                {"type": "AddArtifact", "artifactId": "retroarch_apk", "url": "https://example.com/RetroArch.apk"},
                AddArtifactCommand(artifact_id="retroarch_apk", url="https://example.com/RetroArch.apk"),
            ),
            (
                {
                    "type": "UpdateArtifactField",
                    "artifactId": "retroarch_apk",
                    "field": "url",
                    "value": "https://example.com/new.apk",
                },
                UpdateArtifactFieldCommand(
                    artifact_id="retroarch_apk",
                    field="url",
                    value="https://example.com/new.apk",
                ),
            ),
            (
                {"type": "RenameArtifact", "artifactId": "retroarch_apk", "newArtifactId": "retroarch_release_apk"},
                RenameArtifactCommand(artifact_id="retroarch_apk", new_artifact_id="retroarch_release_apk"),
            ),
            (
                {"type": "DeleteArtifact", "artifactId": "retroarch_apk"},
                DeleteArtifactCommand(artifact_id="retroarch_apk"),
            ),
            (
                {
                    "type": "DuplicateArtifact",
                    "sourceArtifactId": "retroarch_apk",
                    "newArtifactId": "retroarch_apk_copy",
                },
                DuplicateArtifactCommand(source_artifact_id="retroarch_apk", new_artifact_id="retroarch_apk_copy"),
            ),
            ({"type": "AddArtifactGroup", "groupId": "core_bundle"}, AddArtifactGroupCommand(group_id="core_bundle")),
            (
                {"type": "RenameArtifactGroup", "groupId": "core_bundle", "newGroupId": "core_bundle_renamed"},
                RenameArtifactGroupCommand(group_id="core_bundle", new_group_id="core_bundle_renamed"),
            ),
            (
                {
                    "type": "DuplicateArtifactGroup",
                    "sourceGroupId": "core_bundle",
                    "newGroupId": "core_bundle_copy",
                },
                DuplicateArtifactGroupCommand(source_group_id="core_bundle", new_group_id="core_bundle_copy"),
            ),
            (
                {"type": "DeleteArtifactGroup", "groupId": "core_bundle"},
                DeleteArtifactGroupCommand(group_id="core_bundle"),
            ),
            (
                {"type": "ReorderArtifactGroup", "groupId": "core_bundle", "toIndex": 1},
                ReorderArtifactGroupCommand(group_id="core_bundle", to_index=1),
            ),
            (
                {"type": "AddArtifactGroupMember", "groupId": "core_bundle", "artifactId": "core_zip", "index": 0},
                AddArtifactGroupMemberCommand(group_id="core_bundle", artifact_id="core_zip", index=0),
            ),
            (
                {"type": "RemoveArtifactGroupMember", "groupId": "core_bundle", "index": 0},
                RemoveArtifactGroupMemberCommand(group_id="core_bundle", index=0),
            ),
            (
                {"type": "ReorderArtifactGroupMember", "groupId": "core_bundle", "index": 0, "toIndex": 1},
                ReorderArtifactGroupMemberCommand(group_id="core_bundle", index=0, to_index=1),
            ),
            (
                {"type": "AddStep", "stepId": "copy_cores", "stepType": "copy_files", "name": "Copy Cores"},
                AddStepCommand(step_id="copy_cores", step_type="copy_files", name="Copy Cores", index=None),
            ),
            (
                {
                    "type": "AddStep",
                    "stepId": "copy_cores",
                    "stepType": "copy_files",
                    "name": "Copy Cores",
                    "index": 2,
                },
                AddStepCommand(step_id="copy_cores", step_type="copy_files", name="Copy Cores", index=2),
            ),
            (
                {"type": "DeleteStep", "stepId": "copy_cores"},
                DeleteStepCommand(step_id="copy_cores"),
            ),
            (
                {"type": "DuplicateStep", "sourceStepId": "copy_cores", "newStepId": "copy_cores_copy"},
                DuplicateStepCommand(source_step_id="copy_cores", new_step_id="copy_cores_copy"),
            ),
            (
                {"type": "ReorderStep", "stepId": "copy_cores", "toIndex": 0},
                ReorderStepCommand(step_id="copy_cores", to_index=0),
            ),
            (
                {
                    "type": "UpdateStepBasics",
                    "stepId": "copy_cores",
                    "name": "Copy Cores",
                    "description": "Copy staged cores.",
                },
                UpdateStepBasicsCommand(
                    step_id="copy_cores",
                    name="Copy Cores",
                    description="Copy staged cores.",
                ),
            ),
            (
                {
                    "type": "UpdateStepBasics",
                    "stepId": "copy_cores",
                    "name": "Copy Cores",
                    "description": None,
                },
                UpdateStepBasicsCommand(step_id="copy_cores", name="Copy Cores", description=None),
            ),
            (
                {"type": "SetStepUserToggleable", "stepId": "copy_cores", "userToggleable": True},
                SetStepUserToggleableCommand(step_id="copy_cores", user_toggleable=True),
            ),
            (
                {"type": "UpdateStepDependencies", "stepId": "copy_cores", "dependencies": ["extract_cores"]},
                UpdateStepDependenciesCommand(step_id="copy_cores", dependencies=("extract_cores",)),
            ),
            (
                {
                    "type": "UpdateStepParams",
                    "stepId": "copy_cores",
                    "params": {
                        "source": {"ref": "steps.extract_cores.outputs.extracted_paths"},
                        "dest": "/data/user/0/com.retroarch.aarch64/cores",
                        "copy_policy": "merge",
                        "literal_null": None,
                    },
                },
                UpdateStepParamsCommand(
                    step_id="copy_cores",
                    params={
                        "source": RefParamValue(ref="steps.extract_cores.outputs.extracted_paths"),
                        "dest": "/data/user/0/com.retroarch.aarch64/cores",
                        "copy_policy": "merge",
                        "literal_null": None,
                    },
                ),
            ),
            (
                {
                    "type": "UpdateStepParams",
                    "stepId": "grant",
                    "params": {
                        "runtime": [
                            {
                                "package_name": "com.example.app",
                                "name": "POST_NOTIFICATIONS",
                                "when": {"ref": "nested.literal.object"},
                            }
                        ],
                    },
                },
                UpdateStepParamsCommand(
                    step_id="grant",
                    params={
                        "runtime": [
                            {
                                "package_name": "com.example.app",
                                "name": "POST_NOTIFICATIONS",
                                "when": {"ref": "nested.literal.object"},
                            }
                        ],
                    },
                ),
            ),
        ]

        for payload, expected in cases:
            with self.subTest(payload=payload):
                self.assertEqual(decode_recipe_command(payload), expected)

    def test_invalid_command_type_and_payload_raise_controlled_api_error(self) -> None:
        invalid_payloads = [
            {"type": "DeleteRecipe", "recipeId": "example.recipe"},
            {"type": "AddInput"},
            {"type": "AddInput", "inputId": None},
            {"type": "SetOverviewField", "field": "permissions", "value": {}},
            {"type": "UpdateArtifactField", "artifactId": "retroarch_apk", "field": "permissions", "value": {}},
            {"type": "DuplicateArtifact", "sourceArtifactId": "retroarch_apk"},
            {"type": "ReorderArtifactGroupMember", "groupId": "core_bundle", "index": 0, "toIndex": "1"},
            {"type": "DeleteStep"},
            {"type": "DuplicateStep", "stepId": "copy_cores", "newStepId": "copy_cores_copy"},
            {"type": "ReorderStep", "stepId": "copy_cores", "toIndex": "0"},
            {"type": "UpdateStepBasics", "stepId": "copy_cores", "name": "Copy Cores"},
            {"type": "SetStepUserToggleable", "stepId": "copy_cores", "userToggleable": "true"},
            {"type": "UpdateStepDependencies", "stepId": "copy_cores", "dependencies": "extract_cores"},
            {"type": "UpdateStepParams", "stepId": "copy_cores"},
            {"type": "UpdateStepParams", "stepId": "copy_cores", "params": []},
            {"inputId": "roms_dir"},
            "AddInput",
        ]

        for payload in invalid_payloads:
            with self.subTest(payload=payload):
                with self.assertRaises(ApiError) as context:
                    decode_recipe_command(payload)
                self.assertEqual(context.exception.code, "invalid_command")


if __name__ == "__main__":
    unittest.main()
