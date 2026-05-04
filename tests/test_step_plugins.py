from __future__ import annotations

import importlib
import inspect
from pathlib import Path
import re
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import Mock, patch

from emuchef.domain import ExecutionStep, PRIMARY_OUTPUT_STEP_TYPES, STEP_SPECS, ParamMode, ParamSpec, RuntimeValue, RuntimeValueType, StepSpec
from emuchef.io import dump_yaml, load_authored_recipe
from emuchef.io.execution_plan_io import parse_execution_plan
from emuchef.planner import contracts
from emuchef.planner.catalog import CatalogLoadError
from emuchef_editor.app.recipe_editor import step_metadata
from emuchef.steps import builtin as builtin_steps
from emuchef.steps import StepEditorMetadata, StepPlugin, StepRegistry, builtin_step_registry


SUPPORTED_STEP_TYPES = (
    "resolve_artifacts",
    "extract_artifacts",
    "extract_archive",
    "copy_files",
    "install_apk",
    "grant_permissions",
    "launch_app",
    "wait",
    "force_stop_app",
)

HANDLER_MODULES = {
    "resolve_artifacts": "resolve_artifacts",
    "extract_artifacts": "extract_artifacts",
    "extract_archive": "extract_archive",
    "copy_files": "copy_files",
    "install_apk": "install_apk",
    "grant_permissions": "grant_permissions",
    "launch_app": "launch_app",
    "wait": "wait",
    "force_stop_app": "force_stop_app",
}


def _minimal_spec(step_type: str) -> StepSpec:
    return StepSpec(
        type_name=step_type,
        params={"value": ParamSpec(ParamMode.LITERAL, required=False)},
    )


def _minimal_handler(_context, _step, _resolved_params):
    return {}


def _minimal_editor(step_type: str) -> StepEditorMetadata:
    return StepEditorMetadata(label=step_type, param_order=("value",), supported=True)


class StepPluginRegistryTests(unittest.TestCase):
    def test_builtin_registry_contains_exactly_supported_steps(self) -> None:
        registry = builtin_step_registry()

        self.assertEqual(registry.step_types, SUPPORTED_STEP_TYPES)
        self.assertEqual(tuple(plugin.type for plugin in registry.plugins), SUPPORTED_STEP_TYPES)

    def test_every_builtin_plugin_exposes_required_contracts(self) -> None:
        registry = builtin_step_registry()

        for step_type in SUPPORTED_STEP_TYPES:
            plugin = registry.require(step_type)
            self.assertEqual(plugin.type, step_type)
            self.assertIsInstance(plugin.spec, StepSpec)
            self.assertEqual(plugin.spec.type_name, step_type)
            self.assertTrue(callable(plugin.handler))
            self.assertIsNotNone(plugin.editor)
            self.assertEqual(tuple(plugin.editor.param_order), tuple(plugin.spec.params))

    def test_legacy_removed_string_ids_remain_unsupported(self) -> None:
        registry = builtin_step_registry()

        for step_type in ("run_shell", "push_file", "copy_byo_input"):
            self.assertNotIn(step_type, registry)
            with self.assertRaisesRegex(KeyError, step_type):
                registry.require(step_type)

    def test_duplicate_plugin_registration_fails(self) -> None:
        plugin = StepPlugin(
            type="wait",
            spec=_minimal_spec("wait"),
            handler=_minimal_handler,
            editor=_minimal_editor("wait"),
        )

        with self.assertRaisesRegex(ValueError, "Duplicate step plugin"):
            StepRegistry((plugin, plugin))

    def test_missing_spec_or_handler_fails(self) -> None:
        editor = _minimal_editor("wait")

        with self.assertRaisesRegex(ValueError, "missing a StepSpec"):
            StepRegistry((StepPlugin(type="wait", spec=None, handler=_minimal_handler, editor=editor),))

        with self.assertRaisesRegex(ValueError, "missing an executor handler"):
            StepRegistry((StepPlugin(type="wait", spec=_minimal_spec("wait"), handler=None, editor=editor),))

    def test_primary_output_and_step_spec_projections_are_registry_derived(self) -> None:
        registry = builtin_step_registry()

        self.assertEqual(STEP_SPECS, registry.step_specs)
        self.assertEqual(PRIMARY_OUTPUT_STEP_TYPES, registry.primary_output_names)
        self.assertEqual(registry.primary_output_name("extract_artifacts"), "extracted_paths")
        self.assertEqual(registry.primary_output_name("grant_permissions"), None)

    def test_builtin_plugins_reference_step_local_handler_callables_directly(self) -> None:
        registry = builtin_step_registry()

        for step_type, module_name in HANDLER_MODULES.items():
            with self.subTest(step_type=step_type):
                module = importlib.import_module(f"emuchef.steps.handlers.{module_name}")
                self.assertIs(registry.require(step_type).handler, module.handle)

    def test_builtin_registration_has_no_handler_name_indirection(self) -> None:
        source = inspect.getsource(builtin_steps)

        self.assertNotIn("_handler(", source)
        self.assertNotIn("getattr(", source)
        self.assertNotIn("step_handlers", source)
        self.assertNotIn("executor_handler", inspect.getsource(builtin_steps.builtin_step_registry))

    def test_central_step_handler_warehouse_is_removed_from_production(self) -> None:
        project_root = Path(__file__).resolve().parents[1]
        self.assertFalse((project_root / "src" / "emuchef" / "executor" / "step_handlers.py").exists())

        forbidden_patterns = (
            "emuchef.executor import step_handlers",
            "emuchef.executor.step_handlers",
            ".step_handlers import",
            "import step_handlers",
        )
        offenders: list[str] = []
        for path in sorted((project_root / "src").rglob("*.py")):
            if "__pycache__" in path.parts:
                continue
            source = path.read_text(encoding="utf-8")
            if any(pattern in source for pattern in forbidden_patterns):
                offenders.append(str(path.relative_to(project_root)))

        self.assertEqual(offenders, [])

    def test_execute_step_dispatches_only_through_registry_plugin_handler(self) -> None:
        from emuchef.executor import step_runtime

        sentinel_output = {"done": RuntimeValue(type=RuntimeValueType.BOOLEAN, value=True)}
        handler = Mock(return_value=sentinel_output)
        plugin = StepPlugin(
            type="wait",
            spec=_minimal_spec("wait"),
            handler=handler,
            editor=_minimal_editor("wait"),
        )
        registry = StepRegistry((plugin,))
        context = object()
        step = ExecutionStep(id="example.recipe/wait", recipe_ref="example.recipe", type="wait", name="Wait")
        resolved_params = {"duration_ms": 1}

        with patch("emuchef.executor.step_runtime.builtin_step_registry", return_value=registry):
            result = step_runtime.execute_step(context, step, resolved_params)

        self.assertIs(result, sentinel_output)
        handler.assert_called_once_with(context, step, resolved_params)
        source = inspect.getsource(step_runtime.execute_step)
        self.assertNotIn("getattr", source)
        self.assertNotIn("executor_handler", source)
        self.assertNotIn("step_handlers", source)

    def test_production_source_no_longer_depends_on_step_type_enum(self) -> None:
        project_root = Path(__file__).resolve().parents[1]
        forbidden_patterns = (
            re.compile(r"\bclass\s+StepType\b"),
            re.compile(r"\bStepType\."),
            re.compile(r"\bStepType\("),
            re.compile(r"from\s+\.step_types\s+import\s+StepType\b"),
            re.compile(r"from\s+emuchef\.domain\.step_types\s+import\s+StepType\b"),
            re.compile(r"import\s+StepType\b"),
        )
        offenders: list[str] = []

        for path in sorted((project_root / "src").rglob("*.py")):
            if "__pycache__" in path.parts:
                continue
            source = path.read_text(encoding="utf-8")
            if any(pattern.search(source) for pattern in forbidden_patterns):
                offenders.append(str(path.relative_to(project_root)))

        self.assertEqual(offenders, [])

    def test_authored_yaml_round_trips_with_string_type_text_unchanged(self) -> None:
        with TemporaryDirectory() as tmp:
            recipe_path = Path(tmp) / "recipe.yaml"
            recipe_path.write_text(
                """
schema_version: 1
kind: recipe
id: example.recipe
name: Example Recipe
description: Example description.
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
- id: wait
  type: wait
  name: Wait
  user_toggleable: false
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms: 1
  verify: []
""".lstrip(),
                encoding="utf-8",
            )

            recipe = load_authored_recipe(recipe_path)
            rendered = dump_yaml(recipe)

        self.assertEqual(recipe.steps[0].type, "wait")
        self.assertIn("type: wait", rendered)
        self.assertNotIn("StepType", rendered)

    def test_load_authored_recipe_rejects_unsupported_string_step_ids(self) -> None:
        with TemporaryDirectory() as tmp:
            recipe_path = Path(tmp) / "recipe.yaml"
            recipe_path.write_text(
                """
schema_version: 1
kind: recipe
id: example.recipe
name: Example Recipe
description: Example description.
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
- id: custom
  type: custom_plugin_step
  name: Custom
  user_toggleable: false
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  verify: []
""".lstrip(),
                encoding="utf-8",
            )

            with self.assertRaises(CatalogLoadError) as context:
                load_authored_recipe(recipe_path)

        self.assertIn("Unsupported step type 'custom_plugin_step'", context.exception.errors[0].message)

    def test_execution_plan_yaml_round_trips_with_string_type_text_unchanged(self) -> None:
        raw_plan = {
            "schema_version": 1,
            "kind": "execution_plan",
            "id": "plan.test",
            "source": {
                "device_profile_ref": "example.device_profile",
                "device_plan_ref": "example.device_plan",
                "selected_recipe_refs": ["example.recipe"],
                "expanded_recipe_refs": ["example.recipe"],
            },
            "device_context": {
                "manufacturer": "Example",
                "model": "Example",
                "android_version": 13,
                "android_api_level": 33,
                "device_tags": [],
            },
            "runtime_capabilities": {
                "adb_available": True,
                "apk_install": True,
                "shared_storage_write": True,
                "app_launch": True,
                "shell_command": True,
                "package_remove_for_user": False,
                "root_shell": False,
                "app_data_write": False,
            },
            "inputs": [],
            "artifacts": [],
            "steps": [
                {
                    "id": "example.recipe/wait",
                    "recipe_ref": "example.recipe",
                    "type": "wait",
                    "name": "Wait",
                    "params": {"duration_ms": 1},
                }
            ],
        }

        plan = parse_execution_plan(raw_plan)
        rendered = dump_yaml(plan)

        self.assertEqual(plan.steps[0].type, "wait")
        self.assertIn("type: wait", rendered)
        self.assertNotIn("StepType", rendered)

    def test_execution_plan_rejects_unsupported_string_step_ids(self) -> None:
        raw_plan = {
            "schema_version": 1,
            "kind": "execution_plan",
            "id": "plan.test",
            "source": {
                "device_profile_ref": "example.device_profile",
                "device_plan_ref": "example.device_plan",
            },
            "device_context": {
                "manufacturer": "Example",
                "model": "Example",
                "android_version": 13,
                "android_api_level": 33,
                "device_tags": [],
            },
            "runtime_capabilities": {
                "adb_available": True,
                "apk_install": True,
                "shared_storage_write": True,
                "app_launch": True,
                "shell_command": True,
                "package_remove_for_user": False,
                "root_shell": False,
                "app_data_write": False,
            },
            "inputs": [],
            "artifacts": [],
            "steps": [
                {
                    "id": "example.recipe/legacy",
                    "recipe_ref": "example.recipe",
                    "type": "run_shell",
                    "name": "Legacy Shell",
                }
            ],
        }

        with self.assertRaisesRegex(ValueError, "Unsupported step type 'run_shell'"):
            parse_execution_plan(raw_plan)

    def test_executor_dispatch_uses_registry_not_step_type_branching(self) -> None:
        from emuchef.executor import step_runtime

        source = inspect.getsource(step_runtime.execute_step)

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
        sources = [inspect.getsource(builtin_steps)]
        for module_name in HANDLER_MODULES.values():
            module = importlib.import_module(f"emuchef.steps.handlers.{module_name}")
            sources.append(inspect.getsource(module))
        source = "\n".join(sources)

        self.assertNotIn("parse_reference", source)
        self.assertNotIn("RefParamValue", source)


if __name__ == "__main__":
    unittest.main()
