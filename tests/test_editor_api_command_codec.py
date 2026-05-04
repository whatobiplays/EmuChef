from __future__ import annotations

import unittest

from emuchef_editor.api.command_codec import decode_recipe_command
from emuchef_editor.api.errors import ApiError
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddInputCommand,
    AddStepCommand,
    RenameInputCommand,
    SetOverviewFieldCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdateStepDependenciesCommand,
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
            ({"type": "AddArtifactGroup", "groupId": "core_bundle"}, AddArtifactGroupCommand(group_id="core_bundle")),
            (
                {"type": "AddArtifactGroupMember", "groupId": "core_bundle", "artifactId": "core_zip", "index": 0},
                AddArtifactGroupMemberCommand(group_id="core_bundle", artifact_id="core_zip", index=0),
            ),
            (
                {"type": "AddStep", "stepId": "copy_cores", "stepType": "copy_files", "name": "Copy Cores"},
                AddStepCommand(step_id="copy_cores", step_type="copy_files", name="Copy Cores", index=None),
            ),
            (
                {"type": "UpdateStepDependencies", "stepId": "copy_cores", "dependencies": ["extract_cores"]},
                UpdateStepDependenciesCommand(step_id="copy_cores", dependencies=("extract_cores",)),
            ),
        ]

        for payload, expected in cases:
            with self.subTest(payload=payload):
                self.assertEqual(decode_recipe_command(payload), expected)

    def test_invalid_command_type_and_payload_raise_controlled_api_error(self) -> None:
        invalid_payloads = [
            {"type": "DeleteInput", "inputId": "roms_dir"},
            {"type": "AddInput"},
            {"type": "AddInput", "inputId": None},
            {"type": "SetOverviewField", "field": "permissions", "value": {}},
            {"type": "UpdateArtifactField", "artifactId": "retroarch_apk", "field": "permissions", "value": {}},
            {"type": "UpdateStepDependencies", "stepId": "copy_cores", "dependencies": "extract_cores"},
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
