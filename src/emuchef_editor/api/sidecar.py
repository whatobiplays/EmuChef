"""Persistent JSON Lines sidecar for the editor API."""

from __future__ import annotations

from collections.abc import Callable, Mapping
import json
import sys
from typing import Any, TextIO

from .errors import ApiError
from .protocol import failure
from .session import DocumentSessionManager


class JsonlSidecar:
    """Dispatch JSONL requests to a long-lived document session manager.

    The sidecar protocol is intentionally independent of Python implementation
    details. Requests and responses are plain JSON objects, request ids are
    opaque strings owned by the caller, and all document state is exposed
    through stable DTO envelopes.
    """

    def __init__(self, manager: DocumentSessionManager | None = None) -> None:
        self._manager = manager or DocumentSessionManager()

    def handle_request(self, request: Mapping[str, Any] | object) -> dict[str, Any]:
        request_id: str | None = None
        request_type: str | None = None
        debug = False
        try:
            if not isinstance(request, Mapping):
                raise ApiError("invalid_request", "Request must be a JSON object.")
            request_id_value = request.get("id")
            if not isinstance(request_id_value, str) or not request_id_value:
                raise ApiError("invalid_request", "Sidecar request must include a non-empty string id.")
            request_id = request_id_value

            request_type_value = request.get("type")
            if not isinstance(request_type_value, str) or not request_type_value:
                raise ApiError("invalid_request", "Request must include a string type.")
            request_type = request_type_value
            debug = bool(request.get("debug", False))

            payload = request.get("payload", {})
            if payload is None:
                payload = {}
            if not isinstance(payload, Mapping):
                raise ApiError("invalid_request", "Request payload must be an object.")

            response = self._dispatch(request_type, payload)
        except ApiError as exc:
            response = failure(exc, request_type=request_type, debug=debug, exc=exc)
        except Exception as exc:
            response = failure(
                ApiError("internal_error", f"Sidecar request failed: {exc}"),
                request_type=request_type,
                debug=debug,
                exc=exc,
            )
        return _with_id(response, request_id)

    def _dispatch(self, request_type: str, payload: Mapping[str, Any]) -> dict[str, Any]:
        handlers: dict[str, Callable[[Mapping[str, Any]], dict[str, Any]]] = {
            "listStepSpecs": self._list_step_specs,
            "openRecipe": self._open_recipe,
            "createRecipeFromTemplate": self._create_recipe_from_template,
            "closeDocument": self._close_document,
            "getDocument": self._get_document,
            "saveRecipe": self._save_recipe,
            "saveRecipeAs": self._save_recipe_as,
            "emitYaml": self._emit_yaml,
            "applyRecipeCommand": self._apply_recipe_command,
            "undo": self._undo,
            "redo": self._redo,
            "validate": self._validate,
            "getRefIndex": self._get_ref_index,
        }
        handler = handlers.get(request_type)
        if handler is None:
            raise ApiError("invalid_request", f"Unknown request type: {request_type}", {"requestType": request_type})
        return handler(payload)

    def _list_step_specs(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.list_step_specs()

    def _open_recipe(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.open_recipe(
            _required_str(payload, "path"),
            authored_root=_optional_str(payload, "authoredRoot"),
        )

    def _create_recipe_from_template(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.create_recipe_from_template(
            _required_str(payload, "templatePath"),
            destination_path=_required_str(payload, "destinationPath"),
            recipe_id=_required_str(payload, "recipeId"),
            authored_root=_optional_str(payload, "authoredRoot"),
        )

    def _close_document(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.close_document(_required_str(payload, "documentId"))

    def _get_document(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.get_document(_required_str(payload, "documentId"))

    def _save_recipe(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.save_recipe(_required_str(payload, "documentId"))

    def _save_recipe_as(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.save_recipe_as(
            _required_str(payload, "documentId"),
            _required_str(payload, "path"),
        )

    def _emit_yaml(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.emit_yaml(_required_str(payload, "documentId"))

    def _apply_recipe_command(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.apply_recipe_command(
            _required_str(payload, "documentId"),
            payload.get("command"),
        )

    def _undo(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.undo(_required_str(payload, "documentId"))

    def _redo(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.redo(_required_str(payload, "documentId"))

    def _validate(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.validate(_required_str(payload, "documentId"))

    def _get_ref_index(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        return self._manager.get_ref_index(_required_str(payload, "documentId"))


def run_jsonl_sidecar(
    *,
    input_stream: TextIO | None = None,
    output_stream: TextIO | None = None,
    error_stream: TextIO | None = None,
    sidecar: JsonlSidecar | None = None,
) -> int:
    """Run the JSONL sidecar loop until stdin reaches EOF."""

    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    error_stream = error_stream or sys.stderr
    sidecar = sidecar or JsonlSidecar()

    for line in input_stream:
        try:
            request = json.loads(line)
        except json.JSONDecodeError as exc:
            response = _with_id(
                failure(
                    ApiError(
                        "invalid_request",
                        "Malformed JSON line",
                        {"position": exc.pos},
                    ),
                    exc=exc,
                ),
                None,
            )
        else:
            response = sidecar.handle_request(request)

        try:
            output_stream.write(json.dumps(response, sort_keys=True, ensure_ascii=False))
            output_stream.write("\n")
            output_stream.flush()
        except BrokenPipeError:
            error_stream.write("Sidecar stdout pipe closed while writing a response.\n")
            error_stream.flush()
            return 1

    return 0


def _with_id(response: Mapping[str, Any], request_id: str | None) -> dict[str, Any]:
    return {"id": request_id, **dict(response)}


def _required(payload: Mapping[str, Any], field: str) -> Any:
    if field not in payload:
        raise ApiError("invalid_request", f"Request payload is missing required field: {field}", {"field": field})
    return payload[field]


def _required_str(payload: Mapping[str, Any], field: str) -> str:
    value = _required(payload, field)
    if not isinstance(value, str) or not value:
        raise ApiError("invalid_request", f"Request field {field!r} must be a non-empty string.", {"field": field})
    return value


def _optional_str(payload: Mapping[str, Any], field: str) -> str | None:
    value = payload.get(field)
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise ApiError("invalid_request", f"Request field {field!r} must be a non-empty string when provided.", {"field": field})
    return value
