"""Strict planner-side step contracts."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import Enum

from emuchef.domain import BoundParamValue, CopyPolicy, ErrorCode, ErrorMessage, LiteralParamValue, Recipe, Step, StepType


class ParamMode(str, Enum):
    LITERAL_ONLY = "literal_only"
    REF_ONLY = "ref_only"
    BINDABLE = "bindable"


@dataclass(frozen=True, slots=True)
class ParamContract:
    mode: ParamMode
    required: bool = True


@dataclass(frozen=True, slots=True)
class StepContract:
    params: Mapping[str, ParamContract]


STEP_CONTRACTS: dict[StepType, StepContract] = {
    StepType.INSTALL_APK: StepContract(
        params={
            "app": ParamContract(ParamMode.LITERAL_ONLY),
            "replace_existing": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.COPY_BYO_INPUT: StepContract(
        params={
            "input": ParamContract(ParamMode.REF_ONLY),
            "dest": ParamContract(ParamMode.BINDABLE),
            "copy_policy": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.PUSH_FILE: StepContract(
        params={
            "source": ParamContract(ParamMode.BINDABLE),
            "dest": ParamContract(ParamMode.BINDABLE),
            "copy_policy": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.PUSH_DIR: StepContract(
        params={
            "source": ParamContract(ParamMode.BINDABLE),
            "dest": ParamContract(ParamMode.BINDABLE),
            "copy_policy": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.PULL_FILE: StepContract(
        params={
            "source": ParamContract(ParamMode.BINDABLE),
            "dest": ParamContract(ParamMode.BINDABLE),
            "overwrite": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.LAUNCH_APP: StepContract(
        params={
            "package_name": ParamContract(ParamMode.BINDABLE),
            "activity": ParamContract(ParamMode.BINDABLE, required=False),
        }
    ),
    StepType.GRANT_PERMISSIONS: StepContract(params={}),
    StepType.WAIT: StepContract(
        params={
            "duration_ms": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.FORCE_STOP_APP: StepContract(
        params={
            "package_name": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
    StepType.RUN_SHELL: StepContract(
        params={
            "command": ParamContract(ParamMode.BINDABLE),
            "require_root": ParamContract(ParamMode.LITERAL_ONLY),
        }
    ),
}


def validate_step_contract(recipe_ref: str, step: Step) -> tuple[ErrorMessage, ...]:
    contract = STEP_CONTRACTS.get(step.type)
    if contract is None:
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unsupported step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "step_type": step.type.value},
            ),
        )

    errors: list[ErrorMessage] = []
    provided = set(step.params)
    expected = set(contract.params)

    for param_name in sorted(provided - expected):
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unexpected param {param_name!r} for step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
        )

    for param_name, param_contract in contract.params.items():
        if param_contract.required and param_name not in step.params:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Missing required param {param_name!r} for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )
            continue

        if param_name not in step.params:
            continue

        value = step.params[param_name]
        if param_contract.mode is ParamMode.LITERAL_ONLY and isinstance(value, (LiteralParamValue, BoundParamValue)):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Param {param_name!r} must remain a raw literal for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )
        if param_contract.mode is ParamMode.REF_ONLY and not isinstance(value, BoundParamValue):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Param {param_name!r} must be a ref object for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )
        if param_contract.mode is ParamMode.BINDABLE and not isinstance(value, (LiteralParamValue, BoundParamValue)):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Param {param_name!r} must use exactly one of {{value}} or {{ref}} for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )
        if param_name == "copy_policy" and not isinstance(value, (LiteralParamValue, BoundParamValue)):
            try:
                CopyPolicy(str(value))
            except ValueError:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                        message=(
                            f"Param {param_name!r} must be one of: "
                            f"{', '.join(policy.value for policy in CopyPolicy)}."
                        ),
                        details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                    )
                )
        if step.type is StepType.WAIT and param_name == "duration_ms":
            if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                        message="Param 'duration_ms' must be a positive integer for step type 'wait'.",
                        details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                    )
                )
        if step.type is StepType.FORCE_STOP_APP and param_name == "package_name":
            if not isinstance(value, str) or not value.strip():
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                        message="Param 'package_name' must be a non-empty string for step type 'force_stop_app'.",
                        details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                    )
                )

    return tuple(errors)


def validate_recipe_permission_steps(recipe: Recipe) -> tuple[ErrorMessage, ...]:
    has_permissions = bool(recipe.permissions.runtime or recipe.permissions.appops or recipe.permissions.manual)
    if has_permissions:
        return ()

    errors: list[ErrorMessage] = []
    for step in recipe.steps:
        if step.type is not StepType.GRANT_PERMISSIONS:
            continue
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=(
                    f"Step {step.id!r} uses 'grant_permissions' but recipe {recipe.id!r} does not declare local permissions."
                ),
                details={"recipe_ref": recipe.id, "step_id": step.id},
            )
        )
    return tuple(errors)


def referenced_bindings(step: Step) -> tuple[str, ...]:
    refs: list[str] = []
    for value in step.params.values():
        if isinstance(value, BoundParamValue):
            refs.append(value.ref.full)
    return tuple(refs)
