"""The environment for Git commands that build test fixtures in a temporary repository.

Git hooks export the repository they run in. A fixture must not inherit those variables, or its
commands would write to that repository (its index, its objects) instead of its own.
"""

import os

REPOSITORY_VARIABLES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
)


def git_environment() -> dict[str, str]:
    return {key: value for key, value in os.environ.items() if key not in REPOSITORY_VARIABLES}
