#!/usr/bin/env python3
"""Check or update the Rust backend StepSpec fixture from Python DTO metadata.

This script is developer/golden tooling. It imports the Python editor DTO
projection directly, emits the Rust fixture's committed JSON shape, and never
participates in the Tauri runtime path.
"""

from __future__ import annotations

import argparse
import difflib
import json
from pathlib import Path
import shlex
import sys
import tempfile
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE_RELATIVE_PATH = Path("crates/emuchef-rust-backend/tests/fixtures/python_step_specs.json")
MAX_DIFF_LINES = 200


def default_fixture_path() -> Path:
    """Return the committed Rust backend StepSpec fixture path."""

    return REPO_ROOT / DEFAULT_FIXTURE_RELATIVE_PATH


def generate_fixture_obj() -> dict[str, Any]:
    """Build the exact JSON object stored in the Rust StepSpec fixture."""

    from emuchef_editor.api.dto import step_specs_to_dto

    return {"stepSpecs": step_specs_to_dto()}


def canonical_json_text(obj: object) -> str:
    """Serialize fixture data in the stable committed JSON format."""

    return json.dumps(obj, indent=2, sort_keys=True) + "\n"


def read_text(path: Path) -> str:
    """Read UTF-8 text from a fixture path."""

    return path.read_text(encoding="utf-8")


def write_if_changed(path: Path, text: str) -> bool:
    """Write text only when it differs from the current file content."""

    current = path.read_text(encoding="utf-8") if path.exists() else None
    if current == text:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return True


def diff_text(expected: str, actual: str, expected_label: str, actual_label: str) -> str:
    """Return a bounded unified diff for fixture drift output."""

    lines = list(
        difflib.unified_diff(
            expected.splitlines(),
            actual.splitlines(),
            fromfile=expected_label,
            tofile=actual_label,
            lineterm="",
            n=3,
        )
    )
    if len(lines) > MAX_DIFF_LINES:
        omitted = len(lines) - MAX_DIFF_LINES
        lines = lines[:MAX_DIFF_LINES] + [f"... diff truncated, {omitted} additional lines omitted ..."]
    return "\n".join(lines)


def cmd_check(args: argparse.Namespace) -> int:
    """Compare generated StepSpec fixture JSON with the target fixture."""

    fixture_path = args.fixture.resolve()
    generated_text = canonical_json_text(generate_fixture_obj())

    with tempfile.NamedTemporaryFile(
        "w",
        encoding="utf-8",
        delete=False,
        prefix="emuchef-python-step-specs-",
        suffix=".json",
    ) as generated_file:
        generated_file.write(generated_text)
        generated_path = Path(generated_file.name)

    expected_text = read_text(fixture_path)
    print(f"StepSpec fixture: {fixture_path}")
    print(f"Generated output: {generated_path}")
    print(f"Regenerate with: {regeneration_command(fixture_path)}")

    if expected_text == generated_text:
        print("StepSpec fixture check: unchanged")
        return 0

    print("StepSpec fixture check: drift detected", file=sys.stderr)
    diff = diff_text(
        expected_text,
        generated_text,
        expected_label=str(fixture_path),
        actual_label="generated StepSpec fixture",
    )
    if diff:
        print(diff, file=sys.stderr)
    return 1


def cmd_write(args: argparse.Namespace) -> int:
    """Regenerate the target StepSpec fixture when content has changed."""

    fixture_path = args.fixture.resolve()
    generated_text = canonical_json_text(generate_fixture_obj())
    changed = write_if_changed(fixture_path, generated_text)
    print(f"StepSpec fixture: {fixture_path}")
    print(f"StepSpec fixture write: {'updated' if changed else 'unchanged'}")
    return 0


def regeneration_command(fixture_path: Path) -> str:
    """Return the documented command that intentionally updates the fixture."""

    command = [
        "PYTHONPATH=src",
        "./.venv/bin/python",
        "scripts/step_specs_fixture.py",
        "write",
    ]
    if fixture_path != default_fixture_path().resolve():
        command.extend(["--fixture", str(fixture_path)])
    return " ".join(shlex.quote(part) for part in command)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--fixture",
        type=Path,
        default=default_fixture_path(),
        help=f"fixture path to check or update (default: {DEFAULT_FIXTURE_RELATIVE_PATH})",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_parser = subparsers.add_parser(
        "check",
        parents=[common],
        help="compare generated StepSpec JSON with the fixture",
    )
    check_parser.set_defaults(func=cmd_check)

    write_parser = subparsers.add_parser(
        "write",
        parents=[common],
        help="regenerate the StepSpec fixture intentionally",
    )
    write_parser.set_defaults(func=cmd_write)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
