"""PySide-free workspace discovery helpers for authored recipe tooling."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from emuchef.io import load_authored_recipe
from emuchef.planner.catalog import CatalogLoadError


@dataclass(frozen=True, slots=True)
class WorkspaceState:
    requested_root: Path
    authored_root: Path
    recipe_files: tuple[Path, ...]
    template_files: tuple[Path, ...]


def open_workspace(root: str | Path) -> WorkspaceState:
    requested_root = Path(root).resolve()
    authored_root = resolve_authored_root(requested_root)
    repo_root = resolve_repo_root(requested_root)
    recipe_files = tuple(sorted(authored_root.joinpath("recipes").glob("*.y*ml")))
    template_files = tuple(_discover_recipe_templates(repo_root))
    return WorkspaceState(
        requested_root=requested_root,
        authored_root=authored_root,
        recipe_files=recipe_files,
        template_files=template_files,
    )


def resolve_authored_root(root: str | Path) -> Path:
    root_path = Path(root).resolve()
    if root_path.name == "authored" and root_path.joinpath("recipes").is_dir():
        return root_path
    authored_root = root_path / "authored"
    if authored_root.joinpath("recipes").is_dir():
        return authored_root
    raise ValueError(f"{root_path} is not a repo root with authored/ or an authored root.")


def resolve_repo_root(root: str | Path) -> Path:
    root_path = Path(root).resolve()
    if root_path.name == "authored" and root_path.joinpath("recipes").is_dir():
        return root_path.parent
    if root_path.joinpath("authored", "recipes").is_dir():
        return root_path
    raise ValueError(f"{root_path} is not a repo root with authored/ or an authored root.")


def _discover_recipe_templates(repo_root: Path) -> tuple[Path, ...]:
    template_root = repo_root / "templates" / "authored"
    if not template_root.is_dir():
        return ()

    recipe_templates: list[Path] = []
    for candidate in sorted(template_root.glob("*.y*ml")):
        try:
            load_authored_recipe(candidate)
        except CatalogLoadError:
            continue
        recipe_templates.append(candidate.resolve())
    return tuple(recipe_templates)
