import json
import os
import selectors
import signal
import subprocess
import time
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "scripts" / "run-isolated-rust-test.mjs"
DEADLINE = 30


def read_line(stream) -> str:
    descriptor = stream.fileno()
    line = b""
    deadline = time.monotonic() + DEADLINE
    with selectors.DefaultSelector() as selector:
        selector.register(descriptor, selectors.EVENT_READ)
        while not line.endswith(b"\n"):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise AssertionError(f"runner printed no complete line within {DEADLINE} seconds")
            byte = os.read(descriptor, 1)
            if not byte:
                break
            line += byte
    return line.decode().strip()


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
            timeout=DEADLINE,
        )

    def test_runner_isolates_home_forwards_arguments_preserves_exit_status_and_cleans_up(self) -> None:
        result = self.run_runner(
            "const fs=require('node:fs');"
            "console.log(JSON.stringify({home:process.env.FLOW_AGENT_HOME,"
            "homeExists:fs.existsSync(process.env.FLOW_AGENT_HOME),"
            "platform:process.platform,"
            "args:process.argv.slice(1)}));"
            "process.exit(23)",
            "first",
            "second",
        )

        self.assertEqual(result.returncode, 23, result.stderr)
        report = json.loads(result.stdout)
        child_home = Path(report["home"])
        self.assertTrue(child_home.is_absolute())
        self.assertEqual(report["homeExists"], report["platform"] != "win32")
        self.assertEqual(report["args"], ["first", "second"])
        self.assertFalse(child_home.parent.exists())
        self.assertNotEqual(report["home"], "parent-home-sentinel")
        self.assertNotEqual(os.environ.get("FLOW_AGENT_HOME"), report["home"])

    @unittest.skipIf(os.name == "nt", "requires POSIX signals")
    def test_runner_forwards_every_signal_until_the_child_exits(self) -> None:
        child = (
            "process.on('SIGINT',()=>console.log('SIGINT'));"
            "process.on('SIGTERM',()=>process.exit(0));"
            "setTimeout(()=>process.exit(99),10000);"
            "console.log(process.env.FLOW_AGENT_HOME)"
        )
        runner = subprocess.Popen(
            ["node", str(RUNNER), "node", "-e", child],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            child_home = Path(read_line(runner.stdout))
            runner.send_signal(signal.SIGINT)
            self.assertEqual(read_line(runner.stdout), "SIGINT")
            runner.send_signal(signal.SIGTERM)
            self.assertEqual(runner.wait(timeout=30), 0)
        finally:
            if runner.poll() is None:
                runner.kill()
                runner.wait()
            runner.stdout.close()
        self.assertTrue(child_home.is_absolute())
        self.assertFalse(child_home.parent.exists())


if __name__ == "__main__":
    unittest.main()
