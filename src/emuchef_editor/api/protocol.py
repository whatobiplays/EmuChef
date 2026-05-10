"""Response envelope helpers for the editor JSON API."""

from __future__ import annotations

from collections.abc import Mapping
import traceback as traceback_module
from typing import Any

from .errors import ApiError, _json_safe

SUPPORTED_PROTOCOL_VERSION = 1

REQUIRED_CAPABILITIES: tuple[str, ...] = (
    "listStepSpecs",
    "openRecipe",
    "getDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "saveRecipe",
    "validate",
    "emitYaml",
    "getRefIndex",
)

OPTIONAL_CAPABILITIES: tuple[str, ...] = (
    "createRecipeFromTemplate",
    "closeDocument",
    "saveRecipeAs",
)

REPORTED_CAPABILITIES: tuple[str, ...] = REQUIRED_CAPABILITIES + OPTIONAL_CAPABILITIES


def hello_result() -> dict[str, Any]:
    """Return backend-agnostic editor protocol compatibility metadata."""

    return {
        "protocolVersion": SUPPORTED_PROTOCOL_VERSION,
        "capabilities": list(REPORTED_CAPABILITIES),
    }


def success(result: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Return the stable success envelope."""

    return {"ok": True, "result": _json_safe(result or {})}


def failure(
    error: ApiError,
    *,
    request_type: str | None = None,
    debug: bool = False,
    exc: BaseException | None = None,
) -> dict[str, Any]:
    """Return the stable failure envelope, with opt-in diagnostic context."""

    response: dict[str, Any] = {"ok": False, "error": error.to_dict()}
    if debug and exc is not None:
        response["debug"] = {
            "requestType": request_type,
            "exceptionType": type(exc).__name__,
            "traceback": "".join(traceback_module.format_exception(type(exc), exc, exc.__traceback__)),
        }
    return response
