"""Permission action construction and reporting helpers."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import ExecutionStep


def permission_actions(resolved_params: Mapping[str, object]) -> list[dict[str, object]]:
    actions: list[dict[str, object]] = []
    for index, item in enumerate(_coerce_mapping_list(resolved_params.get("runtime"))):
        actions.append(
            {
                "kind": "runtime_permission",
                "package_name": str(item["package_name"]),
                "permission": str(item["name"]),
                "required": bool(item.get("required", True)),
                "when": item.get("when"),
                "source_section": f"params.runtime[{index}]",
            }
        )
    for index, item in enumerate(_coerce_mapping_list(resolved_params.get("appops"))):
        actions.append(
            {
                "kind": "appop",
                "package_name": str(item["package_name"]),
                "op": str(item["op"]),
                "desired_mode": str(item["mode"]),
                "required": bool(item.get("required", True)),
                "when": item.get("when"),
                "source_section": f"params.appops[{index}]",
            }
        )
    return actions


def permission_policy(value: object) -> dict[str, object]:
    if not isinstance(value, Mapping):
        return {"on_failure": "warn", "require_all": False}
    return {
        "on_failure": str(value.get("on_failure", "warn")),
        "require_all": bool(value.get("require_all", False)),
    }


def permission_command(action: Mapping[str, object]) -> list[str]:
    if action["kind"] == "runtime_permission":
        return ["adb", "shell", "pm", "grant", str(action["package_name"]), str(action["permission"])]
    if action["kind"] == "appop":
        return [
            "adb",
            "shell",
            "appops",
            "set",
            str(action["package_name"]),
            str(action["op"]),
            str(action["desired_mode"]),
        ]
    raise ValueError(f"Permission action kind {action['kind']!r} does not have an executable command.")


def permission_result_base(step: ExecutionStep, action: Mapping[str, object]) -> dict[str, object]:
    result: dict[str, object] = {
        "step_id": step.id,
        "kind": str(action["kind"]),
        "package_name": str(action["package_name"]),
        "source_recipe_id": step.recipe_ref,
        "source_section": str(action["source_section"]),
    }
    if action["kind"] == "runtime_permission":
        result["permission"] = str(action["permission"])
    if action["kind"] == "appop":
        result["op"] = str(action["op"])
        result["desired_mode"] = str(action["desired_mode"])
    return result


def permission_not_applicable_reason(
    when: object,
    *,
    rooted: bool,
    android_api_level: int | None,
) -> dict[str, str] | None:
    if not isinstance(when, Mapping):
        return None
    required_rooted = when.get("rooted")
    if required_rooted is True and not rooted:
        return {"reason_code": "requires_root", "message": "Device is not rooted."}
    if required_rooted is False and rooted:
        return {"reason_code": "requires_unrooted", "message": "Device is rooted."}

    api_min = when.get("android_api_min")
    api_max = when.get("android_api_max")
    if (api_min is not None or api_max is not None) and android_api_level is None:
        return {"reason_code": "missing_android_api_level", "message": "Device Android API level is unknown."}
    if isinstance(api_min, int) and android_api_level is not None and android_api_level < api_min:
        return {
            "reason_code": "android_api_out_of_range",
            "message": f"Device Android API {android_api_level} is below minimum {api_min}.",
        }
    if isinstance(api_max, int) and android_api_level is not None and android_api_level > api_max:
        return {
            "reason_code": "android_api_out_of_range",
            "message": f"Device Android API {android_api_level} is above maximum {api_max}.",
        }
    return None


def _coerce_mapping_list(value: object | None) -> list[Mapping[str, object]]:
    if value is None:
        return []
    return [item for item in list(value) if isinstance(item, Mapping)]
