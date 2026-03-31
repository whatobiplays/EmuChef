"""Step type enums."""

from enum import Enum


class StepType(str, Enum):
    RESOLVE_ARTIFACTS = "resolve_artifacts"
    EXTRACT_ARTIFACTS = "extract_artifacts"
    EXTRACT_ARCHIVE = "extract_archive"
    COPY_FILES = "copy_files"
    INSTALL_APK = "install_apk"
    COPY_BYO_INPUT = "copy_byo_input"
    PUSH_FILE = "push_file"
    PUSH_DIR = "push_dir"
    PULL_FILE = "pull_file"
    LAUNCH_APP = "launch_app"
    GRANT_PERMISSIONS = "grant_permissions"
    WAIT = "wait"
    FORCE_STOP_APP = "force_stop_app"
    RUN_SHELL = "run_shell"
