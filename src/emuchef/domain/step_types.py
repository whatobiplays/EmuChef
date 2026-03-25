"""Step type enums."""

from enum import Enum


class StepType(str, Enum):
    INSTALL_APK = "install_apk"
    COPY_BYO_INPUT = "copy_byo_input"
    PUSH_FILE = "push_file"
    PUSH_DIR = "push_dir"
    PULL_FILE = "pull_file"
    LAUNCH_APP = "launch_app"
    RUN_SHELL = "run_shell"
