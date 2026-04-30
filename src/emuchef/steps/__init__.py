"""Step plugin registry exports."""

from __future__ import annotations

from .contracts import StepEditorMetadata, StepOutputMetadata, StepPlugin, StepRegistry


def builtin_step_registry() -> StepRegistry:
    from .builtin import builtin_step_registry as _builtin_step_registry

    return _builtin_step_registry()


__all__ = [
    "StepEditorMetadata",
    "StepOutputMetadata",
    "StepPlugin",
    "StepRegistry",
    "builtin_step_registry",
]
