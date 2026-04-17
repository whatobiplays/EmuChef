from __future__ import annotations

import unittest
from unittest.mock import patch

from emuchef.domain import ArtifactCacheMode, InputRole, InputType
from emuchef.domain.constants import SCHEMA_VERSION
from emuchef_editor.app.recipe_editor import tooltips
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
        self.assertIn("read-only", prompt_tooltip("inputs.id"))
        self.assertIn("artifact", prompt_tooltip("artifacts.id"))
        self.assertIn("group", prompt_tooltip("artifact_groups.id"))

    def test_missing_registry_key_returns_no_tooltip(self) -> None:
        self.assertIsNone(field_tooltip("missing.field"))
        self.assertIsNone(prompt_tooltip("missing.prompt"))

    def test_blank_registry_value_is_treated_as_no_tooltip(self) -> None:
        with patch.dict(tooltips.FIELD_TOOLTIPS, {"blank.field": "   "}):
            self.assertIsNone(field_tooltip("blank.field"))
        with patch.dict(tooltips.PROMPT_TOOLTIPS, {"blank.prompt": "\n\t "}):
            self.assertIsNone(prompt_tooltip("blank.prompt"))


if __name__ == "__main__":
    unittest.main()
