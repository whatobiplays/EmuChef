from __future__ import annotations

import unittest

from emuchef.domain import ArtifactCacheMode, InputRole, InputType
from emuchef.domain.constants import SCHEMA_VERSION
from emuchef_editor.app.recipe_editor.tooltips import field_tooltip, prompt_tooltip


class EditorTooltipTests(unittest.TestCase):
    def test_input_type_tooltip_tracks_current_enum_values(self) -> None:
        tooltip = field_tooltip("inputs.type")
        for value in InputType:
            self.assertIn(value.value, tooltip)

    def test_input_role_tooltip_tracks_current_enum_values(self) -> None:
        tooltip = field_tooltip("inputs.role")
        for value in InputRole:
            self.assertIn(value.value, tooltip)

    def test_artifact_cache_tooltip_tracks_current_enum_values(self) -> None:
        tooltip = field_tooltip("artifacts.cache")
        for value in ArtifactCacheMode:
            self.assertIn(value.value, tooltip)

    def test_schema_version_tooltip_tracks_current_schema_version(self) -> None:
        self.assertIn(str(SCHEMA_VERSION), field_tooltip("overview.schema_version"))

    def test_prompt_tooltips_cover_creation_time_ids(self) -> None:
        self.assertIn("read-only", prompt_tooltip("input_id"))
        self.assertIn("artifacts.<id>.<field>", prompt_tooltip("artifact_id"))
        self.assertIn("Group order", prompt_tooltip("artifact_group_id"))


if __name__ == "__main__":
    unittest.main()
