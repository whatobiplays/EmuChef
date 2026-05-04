"""One-shot JSON server entrypoint for the editor API."""

from __future__ import annotations

from collections.abc import Mapping
import json
import sys
from typing import Any
from uuid import uuid4

from emuchef_editor.core.validation.validator_service import ValidatorService
from emuchef_editor.core.yaml.loader import load_recipe_document

from .dto import diagnostic_to_dto, document_to_dto, step_specs_to_dto
from .errors import ApiError
from .protocol import failure, success


def handle_request(request: Mapping[str, Any] | object) -> dict[str, Any]:
    if not isinstance(request, Mapping):
        return failure(ApiError("invalid_request", "Request must be a JSON object."))
    request_type = request.get("type")
    debug = bool(request.get("debug", False))
    if not isinstance(request_type, str) or not request_type:
        return failure(ApiError("invalid_request", "Request must include a string type."))
    payload = request.get("payload", {})
    if payload is None:
        payload = {}
    if not isinstance(payload, Mapping):
        return failure(ApiError("invalid_request", "Request payload must be an object."))

    if request_type == "listStepSpecs":
        return _list_step_specs(request_type=request_type, debug=debug)
    if request_type == "openRecipe":
        return _open_recipe(payload, request_type=request_type, debug=debug)
    if request_type == "validateRecipePath":
        return _validate_recipe_path(payload, request_type=request_type, debug=debug)
    if request_type == "emitRecipeYamlFromPath":
        return _emit_recipe_yaml_from_path(payload, request_type=request_type, debug=debug)
    return failure(ApiError("invalid_request", f"Unknown request type: {request_type}", {"requestType": request_type}))


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    raw_request = args[0] if args else sys.stdin.read()
    try:
        request = json.loads(raw_request)
    except json.JSONDecodeError as exc:
        response = failure(ApiError("invalid_request", f"Invalid JSON request: {exc.msg}", {"position": exc.pos}), exc=exc)
    else:
        response = handle_request(request)
    sys.stdout.write(json.dumps(response, sort_keys=True))
    sys.stdout.write("\n")
    return 0


def _list_step_specs(*, request_type: str, debug: bool) -> dict[str, Any]:
    try:
        return success({"stepSpecs": step_specs_to_dto()})
    except Exception as exc:
        return failure(ApiError("internal_error", f"Failed to list step specs: {exc}"), request_type=request_type, debug=debug, exc=exc)


def _open_recipe(payload: Mapping[str, Any], *, request_type: str, debug: bool) -> dict[str, Any]:
    try:
        path = _required(payload, "path")
        document = load_recipe_document(path, authored_root=payload.get("authoredRoot"))
        return success({"document": document_to_dto(document, document_id=str(uuid4()))})
    except ApiError as exc:
        return failure(exc, request_type=request_type, debug=debug, exc=exc)
    except Exception as exc:
        return failure(ApiError("load_failed", f"Failed to load recipe: {exc}", {"path": payload.get("path")}), request_type=request_type, debug=debug, exc=exc)


def _validate_recipe_path(payload: Mapping[str, Any], *, request_type: str, debug: bool) -> dict[str, Any]:
    try:
        path = _required(payload, "path")
        result = ValidatorService().validate_path(path, authored_root=payload.get("authoredRoot"))
        return success({"diagnostics": [diagnostic_to_dto(diagnostic) for diagnostic in result.diagnostics]})
    except ApiError as exc:
        return failure(exc, request_type=request_type, debug=debug, exc=exc)
    except Exception as exc:
        return failure(ApiError("validation_failed", f"Validation failed: {exc}", {"path": payload.get("path")}), request_type=request_type, debug=debug, exc=exc)


def _emit_recipe_yaml_from_path(payload: Mapping[str, Any], *, request_type: str, debug: bool) -> dict[str, Any]:
    try:
        path = _required(payload, "path")
        document = load_recipe_document(path, authored_root=payload.get("authoredRoot"))
        return success({"yaml": document.to_yaml()})
    except ApiError as exc:
        return failure(exc, request_type=request_type, debug=debug, exc=exc)
    except Exception as exc:
        return failure(ApiError("load_failed", f"Failed to emit recipe YAML: {exc}", {"path": payload.get("path")}), request_type=request_type, debug=debug, exc=exc)


def _required(payload: Mapping[str, Any], field: str) -> Any:
    if field not in payload:
        raise ApiError("invalid_request", f"Request payload is missing required field: {field}", {"field": field})
    return payload[field]


if __name__ == "__main__":
    raise SystemExit(main())
