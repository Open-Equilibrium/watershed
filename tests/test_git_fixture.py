import os
import unittest
from unittest import mock

from git_fixture import REPOSITORY_VARIABLES, git_environment


class GitFixtureEnvironmentTest(unittest.TestCase):
    def test_fixture_git_never_inherits_the_repository_a_hook_runs_in(self) -> None:
        inherited = {name: "/caller/repository" for name in REPOSITORY_VARIABLES}
        with mock.patch.dict(os.environ, inherited):
            environment = git_environment()
        self.assertFalse(set(REPOSITORY_VARIABLES) & set(environment))
        self.assertEqual(environment["PATH"], os.environ["PATH"])


if __name__ == "__main__":
    unittest.main()
