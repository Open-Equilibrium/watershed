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
