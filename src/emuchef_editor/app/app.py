"""Application entrypoint for the Milestone 1 recipe editor shell."""

from __future__ import annotations

import argparse
import sys


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="emuchef-editor")
    parser.add_argument("workspace_root", nargs="?", help="Repo root containing authored/ or the authored root itself.")
    args = parser.parse_args(argv)

    try:
        from PySide6.QtWidgets import QApplication
    except ImportError:
        sys.stderr.write("PySide6 is required to run emuchef-editor. Install project dependencies first.\n")
        return 1

    from .main_window import MainWindow

    qt_argv = ["emuchef-editor", *(argv or [])]
    app = QApplication.instance() or QApplication(qt_argv)
    window = MainWindow(workspace_root=args.workspace_root)
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
