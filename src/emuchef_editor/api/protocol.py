"""Response envelope helpers for the editor JSON API."""

from __future__ import annotations

from collections.abc import Mapping
import traceback as traceback_module
from typing import Any

from .errors import ApiError, _json_safe


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
