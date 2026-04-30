"""Contracts for first-party step plugins.

The registry is the canonical source of supported step behavior. Core models
still use :class:`StepType` identifiers in this pre-external-plugin phase, but
planner, executor, and editor code should ask the registry for step-specific
behavior instead of maintaining parallel step maps.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import Any

from emuchef.domain.runtime_state import RuntimeValueType
from emuchef.domain.step_specs import StepSpec
from emuchef.domain.step_types import StepType

StepHandler = Callable[[Any, Any, Mapping[str, object]], dict[str, object]]
StepValidationHook = Callable[[str, Any, Mapping[str, object], Any | None], tuple[Any, ...]]
StepNormalizationHook = Callable[[Any, Any, Mapping[str, object]], Mapping[str, object]]


@dataclass(frozen=True, slots=True)
class StepOutputMetadata:
    """Describes one resolvable step output exposed after a successful run."""

    name: str
    value_type: RuntimeValueType
    primary: bool = False


@dataclass(frozen=True, slots=True)
class StepEditorMetadata:
    """Qt-free metadata consumed by editor adapters and YAML emitters."""

    label: str
    param_order: tuple[str, ...] = ()
    supported: bool = True
    ref_filters: Mapping[str, tuple[RuntimeValueType, ...]] = field(default_factory=dict)
    tooltip_key_prefix: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "param_order", tuple(self.param_order))
        object.__setattr__(
            self,
            "ref_filters",
            MappingProxyType({str(name): tuple(types) for name, types in self.ref_filters.items()}),
        )


@dataclass(frozen=True, slots=True)
class StepPlugin:
    """A built-in step plugin with planner, executor, and editor metadata."""

    type: StepType
    spec: StepSpec | None
    handler: StepHandler | None
    editor: StepEditorMetadata | None
    outputs: tuple[StepOutputMetadata, ...] = ()
    validate: StepValidationHook | None = None
    normalize: StepNormalizationHook | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "outputs", tuple(self.outputs))

    @property
    def primary_output(self) -> StepOutputMetadata | None:
        primaries = [output for output in self.outputs if output.primary]
        if len(primaries) > 1:
            raise ValueError(f"Step plugin {self.type.value!r} declares multiple primary outputs.")
        return primaries[0] if primaries else None


class StepRegistry:
    """Immutable registry of first-party step plugins."""

    def __init__(self, plugins: Iterable[StepPlugin]) -> None:
        ordered = tuple(plugins)
        by_type: dict[StepType, StepPlugin] = {}
        for plugin in ordered:
            if plugin.type in by_type:
                raise ValueError(f"Duplicate step plugin for {plugin.type.value!r}.")
            if plugin.spec is None:
                raise ValueError(f"Step plugin {plugin.type.value!r} is missing a StepSpec.")
            if plugin.spec.type_name is not plugin.type:
                raise ValueError(
                    f"Step plugin {plugin.type.value!r} has mismatched StepSpec type {plugin.spec.type_name.value!r}."
                )
            if plugin.handler is None:
                raise ValueError(f"Step plugin {plugin.type.value!r} is missing an executor handler.")
            if plugin.editor is None:
                raise ValueError(f"Step plugin {plugin.type.value!r} is missing editor metadata.")
            if plugin.primary_output is not None and plugin.spec.primary_output_name != plugin.primary_output.name:
                raise ValueError(f"Step plugin {plugin.type.value!r} primary output metadata does not match StepSpec.")
            by_type[plugin.type] = plugin
        self._plugins = ordered
        self._by_type = MappingProxyType(by_type)
        self._step_specs = MappingProxyType({plugin.type: plugin.spec for plugin in ordered if plugin.spec is not None})
        self._primary_output_names = MappingProxyType(
            {
                plugin.type: plugin.primary_output.name
                for plugin in ordered
                if plugin.primary_output is not None
            }
        )

    def __contains__(self, step_type: object) -> bool:
        return step_type in self._by_type

    def __iter__(self):
        return iter(self._plugins)

    @property
    def plugins(self) -> tuple[StepPlugin, ...]:
        return self._plugins

    @property
    def step_types(self) -> tuple[StepType, ...]:
        return tuple(plugin.type for plugin in self._plugins)

    @property
    def step_specs(self) -> Mapping[StepType, StepSpec]:
        return self._step_specs

    @property
    def primary_output_names(self) -> Mapping[StepType, str]:
        return self._primary_output_names

    def get(self, step_type: StepType) -> StepPlugin | None:
        return self._by_type.get(step_type)

    def require(self, step_type: StepType) -> StepPlugin:
        plugin = self.get(step_type)
        if plugin is None:
            raise KeyError(f"Unsupported step type {step_type.value!r}.")
        return plugin

    def primary_output_name(self, step_type: StepType) -> str | None:
        plugin = self.get(step_type)
        if plugin is None:
            return None
        primary = plugin.primary_output
        return primary.name if primary is not None else None

    def editor_metadata(self, step_type: StepType) -> StepEditorMetadata | None:
        plugin = self.get(step_type)
        return plugin.editor if plugin is not None else None
