"""Editor-facing wrappers around shared authored validation."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from emuchef.domain import Recipe, ValidationResult, ValidationStatus
from emuchef.io import validate_authored_recipe, validate_authored_path


@dataclass(frozen=True, slots=True)
class Diagnostic:
    severity: str
    code: str
    message: str
    file: str | None
    object_kind: str | None
    object_id: str | None
    field: str | None


@dataclass(frozen=True, slots=True)
class DiagnosticResult:
    status: ValidationStatus
    diagnostics: tuple[Diagnostic, ...]
    shared_result: ValidationResult


class ValidatorService:
    """Keeps editor diagnostics pinned to the shared authored validation path."""

    def validate_recipe(
        self,
        recipe: Recipe,
        *,
        path: str | Path,
        authored_root: str | Path | None = None,
    ) -> DiagnosticResult:
        return self.from_shared_result(
            validate_authored_recipe(recipe, path=path, authored_root=authored_root)
        )

    def validate_path(self, path: str | Path, *, authored_root: str | Path | None = None) -> DiagnosticResult:
        return self.from_shared_result(validate_authored_path(path, authored_root=authored_root))

    @staticmethod
    def from_shared_result(result: ValidationResult) -> DiagnosticResult:
        diagnostics = tuple(
            list(_warning_diagnostics(result)) + list(_error_diagnostics(result))
        )
        return DiagnosticResult(
            status=result.status,
            diagnostics=diagnostics,
            shared_result=result,
        )


def _warning_diagnostics(result: ValidationResult):
    for warning in result.warnings:
        yield Diagnostic(
            severity="warning",
            code=warning.code.value,
            message=warning.message,
            file=_detail_as_str(warning.details.get("file")),
            object_kind=_detail_as_str(warning.details.get("object_kind")),
            object_id=_detail_as_str(warning.details.get("object_id")),
            field=_detail_as_str(warning.details.get("field")),
        )


def _error_diagnostics(result: ValidationResult):
    for error in result.errors:
        yield Diagnostic(
            severity="error",
            code=error.code.value,
            message=error.message,
            file=_detail_as_str(error.details.get("file")),
            object_kind=_detail_as_str(error.details.get("object_kind")),
            object_id=_detail_as_str(error.details.get("object_id")),
            field=_detail_as_str(error.details.get("field")),
        )


def _detail_as_str(value: object) -> str | None:
    if value is None:
        return None
    return str(value)
