"""UI-free JSON API adapters for the authored recipe editor core."""

from __future__ import annotations

from .errors import ApiError
from .session import DocumentSessionManager

__all__ = ["ApiError", "DocumentSessionManager"]
