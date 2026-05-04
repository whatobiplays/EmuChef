"""In-process document sessions for the editor JSON API."""

from __future__ import annotations

from dataclasses import dataclass
from uuid import uuid4

from emuchef_editor.core.documents.recipe_document import RecipeDocument
from emuchef_editor.core.yaml.loader import create_recipe_document_from_template, load_recipe_document

from .command_codec import decode_recipe_command
from .dto import command_result_to_dto, diagnostic_to_dto, document_to_dto, ref_index_to_dto, step_specs_to_dto
from .errors import ApiError
from .protocol import failure, success


@dataclass(slots=True)
class DocumentSession:
    document_id: str
    document: RecipeDocument


class DocumentSessionManager:
    """Owns live editor documents for future sidecar-style API calls."""

    def __init__(self) -> None:
        self._sessions: dict[str, DocumentSession] = {}

    def open_recipe(self, path: str, authored_root: str | None = None) -> dict:
        try:
            document = load_recipe_document(path, authored_root=authored_root)
        except Exception as exc:
            return failure(ApiError("load_failed", f"Failed to load recipe: {exc}", {"path": path}), exc=exc)
        return self._add_document(document)

    def create_recipe_from_template(
        self,
        template_path: str,
        destination_path: str,
        recipe_id: str,
        authored_root: str | None = None,
    ) -> dict:
        try:
            document = create_recipe_document_from_template(
                template_path,
                destination_path=destination_path,
                recipe_id=recipe_id,
                authored_root=authored_root,
            )
        except Exception as exc:
            return failure(
                ApiError(
                    "load_failed",
                    f"Failed to create recipe from template: {exc}",
                    {"templatePath": template_path, "destinationPath": destination_path},
                ),
                exc=exc,
            )
        return self._add_document(document)

    def close_document(self, document_id: str) -> dict:
        session = self._sessions.pop(document_id, None)
        if session is None:
            return self._unknown_document(document_id)
        return success({})

    def get_document(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        return success({"document": document_to_dto(session.document, document_id=session.document_id)})

    def save_recipe(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        try:
            session.document.save()
        except Exception as exc:
            return failure(ApiError("save_failed", f"Failed to save recipe: {exc}", {"documentId": document_id}), exc=exc)
        return success({"document": document_to_dto(session.document, document_id=session.document_id)})

    def save_recipe_as(self, document_id: str, path: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        try:
            session.document.save_as(path)
        except Exception as exc:
            return failure(
                ApiError("save_failed", f"Failed to save recipe as: {exc}", {"documentId": document_id, "path": path}),
                exc=exc,
            )
        return success({"document": document_to_dto(session.document, document_id=session.document_id)})

    def emit_yaml(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        return success({"yaml": session.document.to_yaml()})

    def apply_recipe_command(self, document_id: str, command: dict) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        try:
            decoded_command = decode_recipe_command(command)
        except ApiError as exc:
            return failure(exc, exc=exc)
        try:
            changed = session.document.apply_command(decoded_command)
        except Exception as exc:
            return failure(ApiError("command_failed", f"Command failed: {exc}", {"documentId": document_id}), exc=exc)
        return success(
            {
                "commandResult": command_result_to_dto(changed=changed),
                "document": document_to_dto(session.document, document_id=session.document_id),
            }
        )

    def undo(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        changed = session.document.undo()
        return success(
            {
                "commandResult": command_result_to_dto(changed=changed),
                "document": document_to_dto(session.document, document_id=session.document_id),
            }
        )

    def redo(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        changed = session.document.redo()
        return success(
            {
                "commandResult": command_result_to_dto(changed=changed),
                "document": document_to_dto(session.document, document_id=session.document_id),
            }
        )

    def validate(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        try:
            result = session.document.validate()
        except Exception as exc:
            return failure(ApiError("validation_failed", f"Validation failed: {exc}", {"documentId": document_id}), exc=exc)
        return success({"diagnostics": [diagnostic_to_dto(diagnostic) for diagnostic in result.diagnostics]})

    def get_ref_index(self, document_id: str) -> dict:
        session = self._session(document_id)
        if session is None:
            return self._unknown_document(document_id)
        return success({"refIndex": ref_index_to_dto(session.document.ref_index)})

    def list_step_specs(self) -> dict:
        return success({"stepSpecs": step_specs_to_dto()})

    def _add_document(self, document: RecipeDocument) -> dict:
        document_id = str(uuid4())
        self._sessions[document_id] = DocumentSession(document_id=document_id, document=document)
        return success({"document": document_to_dto(document, document_id=document_id)})

    def _session(self, document_id: str) -> DocumentSession | None:
        return self._sessions.get(document_id)

    @staticmethod
    def _unknown_document(document_id: str) -> dict:
        return failure(
            ApiError(
                "unknown_document",
                f"Unknown document id: {document_id}",
                {"documentId": document_id},
            )
        )
