"""Binding table construction and validation."""

from __future__ import annotations

import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from emuchef.domain import ErrorCode, ErrorMessage, InputDeclaration, InputType, JSONValue, parse_reference

logger = logging.getLogger(__name__)


class BindingSource(str, Enum):
    USER = "user"
    PLANNER_OVERRIDE = "planner_override"
    DEFAULT = "default"


@dataclass(frozen=True, slots=True)
class BindingEntry:
    id: str
    value: JSONValue
    source: BindingSource


class PlannerOverrideProblemKind(str, Enum):
    INVALID_REF = "invalid_ref"
    METADATA_NOT_ALLOWED = "metadata_not_allowed"
    UNKNOWN_BINDING = "unknown_binding"


@dataclass(frozen=True, slots=True)
class PlannerOverrideProblem:
    key: object
    kind: PlannerOverrideProblemKind
    binding_ref: str | None = None


def normalize_planner_overrides(
    raw_overrides: Mapping[object, JSONValue],
    declarations: Mapping[str, InputDeclaration],
    *,
    allow_metadata_keys: bool,
) -> tuple[dict[str, JSONValue], tuple[PlannerOverrideProblem, ...]]:
    # Only direct `<scope>.$<name>` keys affect binding resolution.
    # `device_plan.overrides.config_variants` remains metadata-only unless wired later.
    result: dict[str, JSONValue] = {}
    problems: list[PlannerOverrideProblem] = []
    for key, value in raw_overrides.items():
        if not isinstance(key, str):
            problems.append(PlannerOverrideProblem(key=key, kind=PlannerOverrideProblemKind.INVALID_REF))
            continue
        if ".$" not in key:
            if allow_metadata_keys:
                continue
            problems.append(PlannerOverrideProblem(key=key, kind=PlannerOverrideProblemKind.METADATA_NOT_ALLOWED))
            continue
        try:
            reference = parse_reference(key)
        except ValueError:
            problems.append(PlannerOverrideProblem(key=key, kind=PlannerOverrideProblemKind.INVALID_REF))
            continue
        if reference.full not in declarations:
            problems.append(
                PlannerOverrideProblem(
                    key=key,
                    kind=PlannerOverrideProblemKind.UNKNOWN_BINDING,
                    binding_ref=reference.full,
                )
            )
            continue
        result[reference.full] = value
    return result, tuple(problems)


def resolve_binding_entry(
    input_id: str,
    declaration: InputDeclaration,
    user_bindings: Mapping[str, JSONValue],
    planner_overrides: Mapping[str, JSONValue],
) -> BindingEntry | None:
    if input_id in user_bindings:
        return BindingEntry(
            id=input_id,
            value=_normalize_binding_value(declaration, user_bindings[input_id]),
            source=BindingSource.USER,
        )
    if input_id in planner_overrides:
        return BindingEntry(
            id=input_id,
            value=_normalize_binding_value(declaration, planner_overrides[input_id]),
            source=BindingSource.PLANNER_OVERRIDE,
        )
    if declaration.default is not None:
        return BindingEntry(
            id=input_id,
            value=_normalize_binding_value(declaration, declaration.default),
            source=BindingSource.DEFAULT,
        )
    return None


def build_binding_table(
    input_ids: Sequence[str],
    declarations: Mapping[str, InputDeclaration],
    user_bindings: Mapping[str, JSONValue],
    planner_overrides: Mapping[str, JSONValue],
) -> dict[str, BindingEntry]:
    table: dict[str, BindingEntry] = {}
    for input_id in input_ids:
        declaration = declarations[input_id]
        binding = resolve_binding_entry(input_id, declaration, user_bindings, planner_overrides)
        if binding is not None:
            table[input_id] = binding
            logger.debug("Binding resolved: %s <- %s (%s)", input_id, binding.value, binding.source.value)
        else:
            logger.debug("Binding unresolved: %s", input_id)
    return table


def validate_binding_value(input_id: str, declaration: InputDeclaration, value: JSONValue) -> tuple[ErrorMessage, ...]:
    if declaration.multiple:
        if not isinstance(value, (list, tuple)):
            return (
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input {input_id!r} requires multiple values.",
                    details={"input_id": input_id},
                ),
            )
        values = list(value)
    else:
        if not isinstance(value, str):
            return (
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input {input_id!r} requires a single string path value.",
                    details={"input_id": input_id},
                ),
            )
        values = [value]

    if declaration.required and not values:
        return (
            ErrorMessage(
                code=ErrorCode.BINDING_VALIDATION_FAILED,
                message=f"Input {input_id!r} requires at least one value.",
                details={"input_id": input_id},
            ),
        )

    errors: list[ErrorMessage] = []
    expected_kind = declaration.validation.path_kind or declaration.type

    for raw_value in values:
        if not isinstance(raw_value, str):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input {input_id!r} values must be string paths.",
                    details={"input_id": input_id},
                )
            )
            continue
        path = Path(raw_value).expanduser()
        if declaration.validation.must_exist and not path.exists():
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input path {raw_value!r} does not exist.",
                    details={"input_id": input_id, "path": raw_value},
                )
            )
            continue
        if expected_kind is InputType.FILE and declaration.validation.must_exist and not path.is_file():
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input path {raw_value!r} must be a file.",
                    details={"input_id": input_id, "path": raw_value},
                )
            )
        if expected_kind is InputType.DIRECTORY and declaration.validation.must_exist and not path.is_dir():
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_VALIDATION_FAILED,
                    message=f"Input path {raw_value!r} must be a directory.",
                    details={"input_id": input_id, "path": raw_value},
                )
            )
        if declaration.validation.allowed_extensions and path.suffix:
            allowed = {
                normalized
                for normalized in (_normalize_allowed_extension(extension) for extension in declaration.validation.allowed_extensions)
                if normalized
            }
            if path.suffix.lower() not in allowed:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.BINDING_VALIDATION_FAILED,
                        message=f"Input path {raw_value!r} has an unsupported extension.",
                        details={"input_id": input_id, "path": raw_value},
                    )
                )

    return tuple(errors)


def _normalize_binding_value(declaration: InputDeclaration, value: JSONValue) -> JSONValue:
    expected_kind = declaration.validation.path_kind or declaration.type
    if expected_kind not in {InputType.FILE, InputType.DIRECTORY}:
        return value
    if declaration.multiple:
        if not isinstance(value, (list, tuple)):
            return value
        return [_normalize_path_string(item) for item in value]
    if not isinstance(value, str):
        return value
    return _normalize_path_string(value)


def _normalize_path_string(raw_value: str) -> str:
    path = Path(raw_value).expanduser()
    if path.is_absolute():
        return str(path)
    return str(Path.cwd() / path)


def _normalize_allowed_extension(extension: str) -> str:
    normalized = str(extension).strip().lower()
    if not normalized:
        return ""
    if normalized.startswith("."):
        return normalized
    return f".{normalized}"


def validate_required_bindings(
    input_ids: Sequence[str],
    declarations: Mapping[str, InputDeclaration],
    table: Mapping[str, BindingEntry],
) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    for input_id in input_ids:
        declaration = declarations[input_id]
        binding = table.get(input_id)
        if binding is None:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_MISSING,
                    message=f"Required binding {input_id!r} is missing.",
                    details={"input_id": input_id},
                )
            )
            continue
        errors.extend(validate_binding_value(input_id, declaration, binding.value))
    return tuple(errors)
