"""JSON-safe DTO conversion for editor documents and registry metadata."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import fields, is_dataclass
from enum import Enum
from pathlib import Path
from typing import Any

from emuchef.domain import (
    InputDeclaration,
    LiteralParamValue,
    Recipe,
    RefParamValue,
    RemoteFileArtifact,
    Step,
    StepCondition,
)
from emuchef.steps import builtin_step_registry
from emuchef.steps.contracts import StepPlugin
from emuchef_editor.core.documents.recipe_document import RecipeDocument
from emuchef_editor.core.refs.ref_index import RefCandidate, RefIndex
from emuchef_editor.core.validation.validator_service import Diagnostic


def document_to_dto(document: RecipeDocument, *, document_id: str) -> dict[str, Any]:
    """Project a live recipe document into JSON-safe document state."""

    return {
        "documentId": document_id,
        "path": str(document.path),
        "authoredRoot": str(document.authored_root) if document.authored_root is not None else None,
        "dirty": document.is_dirty,
        "canUndo": document.can_undo,
        "canRedo": document.can_redo,
        "recipe": recipe_to_dto(document.working_recipe),
        "yaml": document.to_yaml(),
        "diagnostics": [diagnostic_to_dto(diagnostic) for diagnostic in document.validation_result.diagnostics],
        "refIndex": ref_index_to_dto(document.ref_index),
    }


def recipe_to_dto(recipe: Recipe) -> dict[str, Any]:
    """Project authored recipe state without deprecated top-level permissions."""

    return {
        "schemaVersion": recipe.schema_version,
        "kind": recipe.kind,
        "id": recipe.id,
        "name": recipe.name,
        "description": recipe.description or "",
        "recipeDependencies": list(recipe.recipe_dependencies),
        "provides": {"features": list(recipe.provides.features)},
        "inputs": {input_id: _input_to_dto(declaration) for input_id, declaration in recipe.inputs.items()},
        "artifacts": {artifact_id: _artifact_to_dto(artifact) for artifact_id, artifact in recipe.artifacts.items()},
        "artifactGroups": {group_id: list(members) for group_id, members in recipe.artifact_groups.items()},
        "steps": [_step_to_dto(step) for step in recipe.steps],
    }


def diagnostic_to_dto(diagnostic: Diagnostic) -> dict[str, Any]:
    return {
        "severity": diagnostic.severity,
        "code": diagnostic.code,
        "message": diagnostic.message,
        "file": diagnostic.file,
        "objectKind": diagnostic.object_kind,
        "objectId": diagnostic.object_id,
        "field": diagnostic.field,
    }


def ref_index_to_dto(ref_index: RefIndex) -> dict[str, Any]:
    return {
        "inputRefs": list(ref_index.input_refs),
        "artifactRefs": list(ref_index.artifact_refs),
        "stepRefs": list(ref_index.step_refs),
        "stepOutputRefs": list(ref_index.step_output_refs),
        "allRefs": list(ref_index.all_refs),
        "candidates": [ref_candidate_to_dto(candidate) for candidate in ref_index.candidates],
    }


def ref_candidate_to_dto(candidate: RefCandidate) -> dict[str, Any]:
    return {
        "ref": candidate.ref,
        "label": candidate.label,
        "valueType": _enum_value(candidate.value_type),
        "sourceKind": candidate.source_kind,
        "sourceId": candidate.source_id,
    }


def step_specs_to_dto() -> list[dict[str, Any]]:
    return [step_spec_to_dto(plugin) for plugin in builtin_step_registry().plugins]


def step_spec_to_dto(plugin: StepPlugin) -> dict[str, Any]:
    spec = plugin.spec
    editor = plugin.editor
    params: dict[str, Any] = {}
    defaults: dict[str, Any] = {}
    if spec is not None:
        for param_name, param_spec in spec.params.items():
            params[param_name] = {
                "mode": _enum_value(param_spec.mode),
                "required": param_spec.required,
                "enumValues": list(param_spec.enum_values),
            }
            if param_spec.default is not None:
                defaults[param_name] = _json_safe(param_spec.default)
    return {
        "type": plugin.type,
        "label": editor.label if editor is not None else plugin.type,
        "supported": editor.supported if editor is not None else True,
        "primaryOutputName": plugin.primary_output.name if plugin.primary_output is not None else None,
        "outputs": [
            {
                "name": output.name,
                "valueType": _enum_value(output.value_type),
                "primary": output.primary,
            }
            for output in plugin.outputs
        ],
        "paramOrder": list(editor.param_order) if editor is not None else [],
        "params": params,
        "defaults": defaults,
        "refFilters": {
            name: [_enum_value(value_type) for value_type in value_types]
            for name, value_types in (editor.ref_filters.items() if editor is not None else ())
        },
    }


def command_result_to_dto(*, changed: bool) -> dict[str, Any]:
    return {"changed": bool(changed)}


def _input_to_dto(declaration: InputDeclaration) -> dict[str, Any]:
    return {
        "id": declaration.id,
        "type": _enum_value(declaration.type),
        "role": _enum_value(declaration.role),
        "label": declaration.label,
        "description": declaration.description or "",
        "required": declaration.required,
        "multiple": declaration.multiple,
        "validation": {
            "mustExist": declaration.validation.must_exist,
            "allowedExtensions": list(declaration.validation.allowed_extensions),
            "pathKind": _enum_value(declaration.validation.path_kind),
        },
        "default": _json_safe(declaration.default),
        "metadata": _json_safe(declaration.metadata),
    }


def _artifact_to_dto(artifact: RemoteFileArtifact) -> dict[str, Any]:
    return {
        "id": artifact.id,
        "type": _enum_value(artifact.type),
        "url": artifact.url,
        "cache": _enum_value(artifact.cache),
    }


def _step_to_dto(step: Step) -> dict[str, Any]:
    return {
        "id": step.id,
        "type": step.type,
        "name": step.name,
        "description": step.description or "",
        "userToggleable": step.user_toggleable,
        "dependencies": list(step.dependencies),
        "constraints": {
            "capabilities": list(step.constraints.capabilities),
            "conflictsWith": list(step.constraints.conflicts_with),
        },
        "skipIf": [_condition_to_dto(condition) for condition in step.skip_if],
        "params": {str(name): _json_safe(value) for name, value in step.params.items()},
        "verify": [_condition_to_dto(condition) for condition in step.verify],
    }


def _condition_to_dto(condition: StepCondition) -> dict[str, Any]:
    return {
        "type": condition.type,
        "params": _json_safe(condition.params),
    }


def _json_safe(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, RefParamValue):
        return {"ref": value.ref}
    if isinstance(value, LiteralParamValue):
        return _json_safe(value.value)
    if isinstance(value, Mapping):
        return {str(key): _json_safe(item) for key, item in value.items()}
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return [_json_safe(item) for item in value]
    if is_dataclass(value):
        return {field.name: _json_safe(getattr(value, field.name)) for field in fields(value)}
    return str(value)


def _enum_value(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, Enum):
        return value.value
    return value
