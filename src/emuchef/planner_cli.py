"""Transitional CLI compatibility boundary for Python planner-owned helpers.

This module keeps src/emuchef/cli.py from importing emuchef.planner directly
while draft/profile/session CLI paths are still being retired or moved during
the Rust planner cutover.
"""

from emuchef.planner import (
    BindInput,
    CatalogLoadError,
    DeselectRecipe,
    DeselectStep,
    Planner,
    ProfileMatchFacts,
    SelectRecipe,
    SelectStep,
    UnbindInput,
    match_device_profile,
    match_device_profiles,
)

__all__ = [
    "BindInput",
    "CatalogLoadError",
    "DeselectRecipe",
    "DeselectStep",
    "Planner",
    "ProfileMatchFacts",
    "SelectRecipe",
    "SelectStep",
    "UnbindInput",
    "match_device_profile",
    "match_device_profiles",
]
