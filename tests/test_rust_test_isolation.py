import json
import os
import selectors
import signal
import subprocess
import sys
import time
import tomllib
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "scripts" / "run-isolated-rust-test.mjs"
DEADLINE = 30


def read_line(stream) -> str:
    """The next line of `stream`; fails instead of waiting past the deadline, even mid-line."""
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

    @unittest.skipIf(os.name == "nt", "requires POSIX pipes")
    def test_read_line_gives_up_on_an_unfinished_line(self) -> None:
        # The child leaves a half line in the pipe and stays alive, so only the deadline ends the read.
        child = subprocess.Popen(
            [sys.executable, "-c",
             "import sys, time; sys.stdout.write('partial'); sys.stdout.flush(); time.sleep(5)"],
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            with mock.patch.object(sys.modules[__name__], "DEADLINE", 1), self.assertRaises(AssertionError):
                read_line(child.stdout)
        finally:
            child.kill()
            child.wait()
            child.stdout.close()

    @unittest.skipIf(os.name == "nt", "requires POSIX signals")
    def test_runner_forwards_every_signal_until_the_child_exits(self) -> None:
        # The child survives SIGINT and exits on SIGTERM; 99 means SIGTERM never arrived.
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
