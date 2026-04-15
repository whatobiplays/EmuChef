"""Workspace discovery helpers for the editor shell."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class WorkspaceState:
    requested_root: Path
    authored_root: Path
    recipe_files: tuple[Path, ...]


def open_workspace(root: str | Path) -> WorkspaceState:
    requested_root = Path(root).resolve()
    authored_root = resolve_authored_root(requested_root)
    recipe_files = tuple(sorted(authored_root.joinpath("recipes").glob("*.y*ml")))
    return WorkspaceState(
        requested_root=requested_root,
        authored_root=authored_root,
        recipe_files=recipe_files,
    )


def resolve_authored_root(root: str | Path) -> Path:
    root_path = Path(root).resolve()
    if root_path.name == "authored" and root_path.joinpath("recipes").is_dir():
        return root_path
    authored_root = root_path / "authored"
    if authored_root.joinpath("recipes").is_dir():
        return authored_root
    raise ValueError(f"{root_path} is not a repo root with authored/ or an authored root.")
