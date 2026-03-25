"""Copy policy values for copy-style step params."""

from enum import Enum


class CopyPolicy(str, Enum):
    MERGE = "merge"
    SYNC = "sync"
    REPLACE = "replace"
