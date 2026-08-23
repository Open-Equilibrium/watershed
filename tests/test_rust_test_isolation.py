import json
import os
import subprocess
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "scripts" / "run-isolated-rust-test.mjs"


class RustTestIsolationRunnerTest(unittest.TestCase):
    def test_cargo_runner_path_resolves_from_every_workspace_member(self) -> None:
        config = tomllib.loads((ROOT / ".cargo" / "test-isolation.toml").read_text())
        runner = config["target"]["cfg(all())"]["runner"]
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())

        self.assertEqual(runner[0], "node")
        for member in workspace["workspace"]["members"]:
            resolved = (ROOT / member / runner[1]).resolve()
            self.assertEqual(resolved, RUNNER, member)

    def run_runner(self, child_source: str, *args: str) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment["FLOW_AGENT_HOME"] = "parent-home-sentinel"
        environment["TEMP"] = str(ROOT)
        environment["TMP"] = str(ROOT)
        return subprocess.run(
            ["node", str(RUNNER), "node", "-e", child_source, *args],
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_runner_isolates_home_forwards_arguments_and_cleans_up(self) -> None:
        result = self.run_runner(
            "const fs=require('node:fs');"
            "console.log(JSON.stringify({home:process.env.FLOW_AGENT_HOME,"
            "homeExists:fs.existsSync(process.env.FLOW_AGENT_HOME),"
            "platform:process.platform,"
            "args:process.argv.slice(1)}))",
            "first",
            "second",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        child_home = Path(report["home"])
        self.assertTrue(child_home.is_absolute())
        self.assertEqual(report["homeExists"], report["platform"] != "win32")
        self.assertEqual(report["args"], ["first", "second"])
        self.assertFalse(child_home.parent.exists())
        self.assertNotEqual(report["home"], "parent-home-sentinel")
        self.assertNotEqual(os.environ.get("FLOW_AGENT_HOME"), report["home"])

    def test_runner_preserves_child_exit_status_and_still_cleans_up(self) -> None:
        result = self.run_runner(
            "console.log(process.env.FLOW_AGENT_HOME); process.exit(23)",
        )

        self.assertEqual(result.returncode, 23, result.stderr)
        child_home = Path(result.stdout.strip())
        self.assertTrue(child_home.is_absolute())
        self.assertFalse(child_home.parent.exists())


if __name__ == "__main__":
    unittest.main()
