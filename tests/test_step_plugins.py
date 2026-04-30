from __future__ import annotations

import inspect
import unittest

from emuchef.domain import PRIMARY_OUTPUT_STEP_TYPES, STEP_SPECS, ParamMode, ParamSpec, StepSpec, StepType
from emuchef.executor import step_handlers
from emuchef.planner import contracts
from emuchef_editor.app.recipe_editor import step_metadata
from emuchef.steps import builtin as builtin_steps
from emuchef.steps import StepEditorMetadata, StepPlugin, StepRegistry, builtin_step_registry


SUPPORTED_STEP_TYPES = (
    StepType.RESOLVE_ARTIFACTS,
    StepType.EXTRACT_ARTIFACTS,
    StepType.EXTRACT_ARCHIVE,
    StepType.COPY_FILES,
    StepType.INSTALL_APK,
    StepType.GRANT_PERMISSIONS,
    StepType.LAUNCH_APP,
    StepType.WAIT,
    StepType.FORCE_STOP_APP,
)


def _minimal_spec(step_type: StepType) -> StepSpec:
    return StepSpec(
        type_name=step_type,
        params={"value": ParamSpec(ParamMode.LITERAL, required=False)},
    )


def _minimal_handler(_context, _step, _resolved_params):
    return {}


def _minimal_editor(step_type: StepType) -> StepEditorMetadata:
    return StepEditorMetadata(label=step_type.value, param_order=("value",), supported=True)


class StepPluginRegistryTests(unittest.TestCase):
    def test_builtin_registry_contains_exactly_supported_steps(self) -> None:
        registry = builtin_step_registry()

        self.assertEqual(registry.step_types, SUPPORTED_STEP_TYPES)
        self.assertEqual(tuple(plugin.type for plugin in registry.plugins), SUPPORTED_STEP_TYPES)

    def test_every_builtin_plugin_exposes_required_contracts(self) -> None:
        registry = builtin_step_registry()

        for step_type in SUPPORTED_STEP_TYPES:
            plugin = registry.require(step_type)
            self.assertIs(plugin.type, step_type)
            self.assertIsInstance(plugin.spec, StepSpec)
            self.assertIs(plugin.spec.type_name, step_type)
            self.assertTrue(callable(plugin.handler))
            self.assertIsNotNone(plugin.editor)
            self.assertEqual(tuple(plugin.editor.param_order), tuple(plugin.spec.params))

    def test_unsupported_enum_values_remain_unsupported(self) -> None:
        registry = builtin_step_registry()

        for step_type in set(StepType) - set(SUPPORTED_STEP_TYPES):
            self.assertNotIn(step_type, registry)
            with self.assertRaisesRegex(KeyError, step_type.value):
                registry.require(step_type)

    def test_duplicate_plugin_registration_fails(self) -> None:
        plugin = StepPlugin(
            type=StepType.WAIT,
            spec=_minimal_spec(StepType.WAIT),
            handler=_minimal_handler,
            editor=_minimal_editor(StepType.WAIT),
        )

        with self.assertRaisesRegex(ValueError, "Duplicate step plugin"):
            StepRegistry((plugin, plugin))

    def test_missing_spec_or_handler_fails(self) -> None:
        editor = _minimal_editor(StepType.WAIT)

        with self.assertRaisesRegex(ValueError, "missing a StepSpec"):
            StepRegistry((StepPlugin(type=StepType.WAIT, spec=None, handler=_minimal_handler, editor=editor),))

        with self.assertRaisesRegex(ValueError, "missing an executor handler"):
            StepRegistry((StepPlugin(type=StepType.WAIT, spec=_minimal_spec(StepType.WAIT), handler=None, editor=editor),))

    def test_primary_output_and_step_spec_projections_are_registry_derived(self) -> None:
        registry = builtin_step_registry()

        self.assertEqual(STEP_SPECS, registry.step_specs)
        self.assertEqual(PRIMARY_OUTPUT_STEP_TYPES, registry.primary_output_names)
        self.assertEqual(registry.primary_output_name(StepType.EXTRACT_ARTIFACTS), "extracted_paths")
        self.assertEqual(registry.primary_output_name(StepType.GRANT_PERMISSIONS), None)

    def test_executor_dispatch_uses_registry_not_step_type_branching(self) -> None:
        source = inspect.getsource(step_handlers.execute_step)

        self.assertNotIn("StepType.", source)
        self.assertNotIn("if step.type", source)

    def test_planner_normalization_uses_plugin_hooks_not_step_type_branching(self) -> None:
        source = inspect.getsource(contracts.normalize_step_params_for_execution)

        self.assertNotIn("StepType.", source)
        self.assertNotIn("if step.type", source)

    def test_planner_validation_uses_plugin_hooks_not_step_type_branching(self) -> None:
        source = inspect.getsource(contracts.validate_step_contract)

        self.assertNotIn("_validate_step_specifics", source)
        self.assertFalse(hasattr(contracts, "_validate_step_specifics"))

    def test_editor_step_metadata_is_registry_derived(self) -> None:
        registry = builtin_step_registry()
        expected_ref_filters = {
            (plugin.type, param_name): allowed_types
            for plugin in registry.plugins
            for param_name, allowed_types in plugin.editor.ref_filters.items()
        }

        self.assertEqual(step_metadata.SUPPORTED_EDITOR_STEP_TYPES, SUPPORTED_STEP_TYPES)
        self.assertEqual(step_metadata.REF_VALUE_FILTERS, expected_ref_filters)
        self.assertNotIn("StepType.", inspect.getsource(step_metadata))

    def test_builtin_plugins_do_not_parse_authored_refs(self) -> None:
        source = inspect.getsource(builtin_steps)

        self.assertNotIn("parse_reference", source)
        self.assertNotIn("RefParamValue", source)


if __name__ == "__main__":
    unittest.main()
