"""Loader helpers for typed authored recipe editor documents."""

from __future__ import annotations

from dataclasses import replace
from pathlib import Path

from emuchef.io import load_authored_recipe

from ..documents.recipe_document import RecipeDocument
from ..validation.validator_service import ValidatorService
from .writer import write_recipe_yaml


def load_recipe_document(
    path: str | Path,
    *,
    authored_root: str | Path | None = None,
    validator_service: ValidatorService | None = None,
) -> RecipeDocument:
    recipe_path = Path(path).resolve()
    normalized_authored_root = _normalize_authored_root(authored_root, recipe_path=recipe_path)
    recipe = load_authored_recipe(recipe_path)
    return RecipeDocument(
        path=recipe_path,
        authored_root=normalized_authored_root,
        working_recipe=recipe,
        validator_service=validator_service,
    )


def create_recipe_document_from_template(
    template_path: str | Path,
    *,
    destination_path: str | Path,
    recipe_id: str,
    authored_root: str | Path | None = None,
    validator_service: ValidatorService | None = None,
) -> RecipeDocument:
    normalized_template_path = Path(template_path).resolve()
    normalized_destination_path = Path(destination_path).resolve()
    normalized_authored_root = _normalize_authored_root(
        authored_root,
        recipe_path=normalized_destination_path,
    )
    template_recipe = load_authored_recipe(normalized_template_path)
    created_recipe = replace(template_recipe, id=recipe_id)
    baseline_yaml = write_recipe_yaml(created_recipe, normalized_destination_path)
    return RecipeDocument(
        path=normalized_destination_path,
        authored_root=normalized_authored_root,
        working_recipe=created_recipe,
        validator_service=validator_service,
        baseline_yaml=baseline_yaml,
    )


def infer_authored_root(recipe_path: str | Path) -> Path | None:
    target_path = Path(recipe_path).resolve()
    for parent in target_path.parents:
        if parent.name == "authored" and (parent / "recipes").is_dir():
            return parent
    return None


def _normalize_authored_root(authored_root: str | Path | None, *, recipe_path: Path) -> Path | None:
    if authored_root is not None:
        root_path = Path(authored_root).resolve()
        authored_candidate = root_path / "authored"
        if root_path.name == "authored" and (root_path / "recipes").is_dir():
            return root_path
        if authored_candidate.is_dir() and (authored_candidate / "recipes").is_dir():
            return authored_candidate
        return root_path
    return infer_authored_root(recipe_path)
