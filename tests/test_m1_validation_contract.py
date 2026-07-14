import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

class M1ValidationContractTest(unittest.TestCase):
    def assert_git_ignore(self, path: str, *, ignored: bool) -> None:
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--quiet", path],
            cwd=ROOT,
        )
        if result.returncode not in (0, 1):
            raise AssertionError(
                f"git check-ignore failed for {path} with {result.returncode}"
            )
        self.assertEqual(result.returncode == 0, ignored, path)

    def test_gitignore_keeps_loop_workspace_config_trackable(self) -> None:
        self.assert_git_ignore(
            "loop-agent/fixtures/new-fixture/.loop/config.yaml",
            ignored=False,
        )
        self.assert_git_ignore(
            "loop-agent/fixtures/new-fixture/.loop/sessions/session.jsonl",
            ignored=True,
        )
        self.assert_git_ignore(
            "loop-agent/fixtures/new-fixture/.loop/logs/session.log",
            ignored=True,
        )

if __name__ == "__main__":
    unittest.main()
