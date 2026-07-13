import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def coverage_ignore_regex(text: str) -> re.Pattern[str]:
    match = re.search(r"--ignore-filename-regex\s+'([^']+)'", text)
    if match is None:
        raise AssertionError("missing --ignore-filename-regex value")
    return re.compile(match.group(1))


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

    def test_ci_enforces_m1_coverage_gate(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("M1 gates", workflow)
        for token in [
            "cargo llvm-cov nextest",
            "--locked",
            "--workspace",
            "--fail-under-lines 90",
        ]:
            self.assertIn(token, workflow)
        self.assertIn("--ignore-filename-regex", workflow)
        ignore = coverage_ignore_regex(workflow)
        for path in [
            "core/core-script/src/tests.rs",
            "core/core-policy/src/tests.rs",
            "proto/proto/src/tests.rs",
            "loop-agent/loop-agent-core/src/tests.rs",
            "loop-agent/loop-agent-core/tests/performance.rs",
        ]:
            self.assertRegex(path, ignore)
        for path in [
            "core/core-script/src/lib.rs",
            "core/core-policy/src/lib.rs",
            "proto/proto/src/lib.rs",
            "loop-agent/loop-agent-core/src/lib.rs",
        ]:
            self.assertNotRegex(path, ignore)
        self.assertIn("--show-missing-lines", workflow)
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", workflow)

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

    def test_ci_trigger_scope_covers_merges_and_feature_pushes(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("pull_request:\n    branches: [main]", workflow)
        self.assertIn('push:\n    branches: [main, "feat/**"]', workflow)


if __name__ == "__main__":
    unittest.main()
