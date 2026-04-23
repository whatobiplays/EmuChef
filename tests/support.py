from __future__ import annotations

from pathlib import Path

import yaml


def write_yaml(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")


def base_recipe(
    *,
    recipe_id: str = "example.recipe",
    name: str = "Example Recipe",
    recipe_dependencies: list[str] | None = None,
    inputs: dict | None = None,
    artifacts: dict | None = None,
    artifact_groups: dict | None = None,
    permissions: dict | None = None,
    steps: list[dict] | None = None,
) -> dict:
    return {
        "schema_version": 1,
        "kind": "recipe",
        "id": recipe_id,
        "name": name,
        "description": f"{name} description.",
        "recipe_dependencies": recipe_dependencies or [],
        "provides": {"features": [recipe_id.replace(".", "_")]},
        "inputs": inputs or {},
        "artifacts": artifacts or {},
        "artifact_groups": artifact_groups or {},
        "permissions": permissions or {},
        "steps": steps or [],
    }


def build_authored_tree(
    root: Path,
    *,
    recipes: list[dict],
    recipe_templates: dict[str, dict] | None = None,
    selected_recipe_refs: list[str] | None = None,
    device_plan_overrides: dict | None = None,
    capability_defaults: dict | None = None,
) -> Path:
    authored_root = root / "authored"
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (authored_root / subdir).mkdir(parents=True, exist_ok=True)

    write_yaml(
        authored_root / "apps" / "example_app.yaml",
        {
            "schema_version": 1,
            "kind": "app_definition",
            "id": "example.app",
            "name": "Example App",
            "description": "Minimal app for tests.",
            "category": "utility",
            "package": {"primary": "com.example.app", "aliases": []},
            "install_source": {"type": "local_file", "resolver": "none", "options": {"path": "sample_artifacts/example.apk"}},
            "tracking_source": {
                "type": "local_metadata",
                "config_snapshot": "vendor/example/app.json",
                "app_id": "example-app",
            },
            "artifacts": {
                "apk": {"required": True},
                "shared_storage_config": {"supported": False},
                "app_data_config": {"supported": False},
                "byo_apk": {"required": False},
            },
            "provisioning": {
                "launch_once_recommended": False,
                "shared_storage_paths": [],
                "app_data_paths": [],
                "config_targets": [],
            },
            "inputs": [],
            "metadata": {},
        },
    )

    for recipe in recipes:
        filename = f"{recipe['id'].replace('.', '_')}.yaml"
        write_yaml(authored_root / "recipes" / filename, recipe)

    write_yaml(
        authored_root / "device_profiles" / "example_device_profile.yaml",
        {
            "schema_version": 1,
            "kind": "device_profile",
            "id": "example.device_profile",
            "name": "Example Device Profile",
            "description": "Minimal device profile for tests.",
            "match": {
                "manufacturer_contains": ["Example"],
                "brand_contains": ["Example"],
                "model_patterns": ["(?i)example"],
                "android_version": {"min": 13},
            },
            "capability_defaults": capability_defaults
            or {
                "adb_available": True,
                "apk_install": True,
                "shared_storage_write": True,
                "app_launch": True,
                "shell_command": True,
                "package_remove_for_user": False,
                "root_shell": True,
                "app_data_write": True,
            },
            "device_tags": ["example_tag"],
            "metadata": {},
        },
    )

    selected = selected_recipe_refs or [recipe["id"] for recipe in recipes]
    write_yaml(
        authored_root / "device_plans" / "example_device_plan.yaml",
        {
            "schema_version": 1,
            "kind": "device_plan",
            "id": "example.device_plan",
            "name": "Example Device Plan",
            "description": "Minimal device plan for tests.",
            "device_profile_ref": "example.device_profile",
            "recipes": [{"recipe_ref": recipe_ref, "selected_by_default": True} for recipe_ref in selected],
            "defaults": {},
            "overrides": device_plan_overrides or {},
            "metadata": {},
        },
    )

    if recipe_templates:
        template_root = root / "templates" / "authored"
        template_root.mkdir(parents=True, exist_ok=True)
        for filename, payload in recipe_templates.items():
            write_yaml(template_root / filename, payload)

    return authored_root
