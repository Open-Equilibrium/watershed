"""Evidence classification must not mistake a broken helper for protection."""

import importlib.util
import json
import os
import plistlib
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "app_probe", Path(__file__).resolve().parents[1] / "scripts" / "probe-macos-app-sandbox.py"
)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class NativeObservation(unittest.TestCase):
    def test_runtime_grant_is_read_only_and_helper_only_inherits(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "fixture"; binary.write_bytes(b"synthetic")
            project, runtime = root / "project", root / "node-runtime"
            with patch.object(PROBE, "require"):
                PROBE.bundle(None, root / "app", binary, True, project, True, runtime_reads=[runtime])
            launcher = plistlib.loads((root / "app/launcher.entitlements").read_bytes())
            helper = plistlib.loads((root / "app/helper.entitlements").read_bytes())
            self.assertEqual(launcher["com.apple.security.temporary-exception.files.absolute-path.read-only"], [str(runtime) + "/"])
            self.assertEqual(launcher["com.apple.security.temporary-exception.files.absolute-path.read-write"], [str(project) + "/"])
            self.assertEqual(helper, {"com.apple.security.app-sandbox": True, "com.apple.security.inherit": True})

    def test_npm_baseline_requires_lifecycle_and_protected_write(self):
        rows = [{"case": "npm_javascript", "observation": "completed", "protected_changed": True,
                 "command": {"output": "npm_protected_write=allowed\n"}},
                {"case": "npm_native-xcrun", "observation": "completed", "protected_changed": True,
                 "command": {"output": "npm_protected_write=allowed\n"}},
                {"case": "npm_native-direct", "observation": "completed", "protected_changed": True,
                 "command": {"output": "npm_protected_write=allowed\n"}}]
        self.assertTrue(PROBE.npm_baseline_completed(rows))
        rows[0]["protected_changed"] = False
        self.assertFalse(PROBE.npm_baseline_completed(rows))

    def test_npm_build_requires_successful_lifecycle_and_verified_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            host = PROBE.npm_host()
            project, command = PROBE.npm_fixture(root, host, "javascript")
            environment = {"PATH": str(Path(host["node"]).parent) + os.pathsep + os.defpath}
            if "SystemRoot" in os.environ:
                environment["SystemRoot"] = os.environ["SystemRoot"]
                environment["ComSpec"] = str(Path(os.environ["SystemRoot"]) / "System32" / "cmd.exe")
                environment["PATH"] += os.pathsep + str(Path(os.environ["SystemRoot"]) / "System32")
            # App Sandbox changes the initial directory; npm must select the fixture explicitly.
            result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True, timeout=30)
            observation = {"launched": True, "returncode": result.returncode, "timeout": False,
                           "output": result.stdout + result.stderr}
            self.assertEqual(result.returncode, 0, observation["output"])
            self.assertEqual(PROBE.classify_npm_build(observation, project), "completed")
            (project / "dist" / "result.json").write_text(json.dumps({"answer": 0}), encoding="utf-8")
            self.assertEqual(PROBE.classify_npm_build(observation, project), "failure")
            observation["returncode"] = 1
            self.assertEqual(PROBE.classify_npm_build(observation, project), "not_completed")
            observation.update(returncode=0, timeout=True)
            self.assertEqual(PROBE.classify_npm_build(observation, project), "not_completed")

    def test_permission_denial_requires_started_helper_explicit_errno_and_intact_file(self):
        for code, output, changed, timeout, expected in (
            (0, "probe_started\n", True, False, "allowed"),
            (10, "probe_started\nprobe_errno=1\n", False, False, "denied"),
            (10, "probe_started\nprobe_errno=13\n", False, False, "denied"),
            (10, "probe_errno=1\n", False, False, "failure"),
            (10, "probe_started\nprobe_errno=2\n", False, False, "failure"),
            (10, "probe_started\nprobe_errno=1\n", True, False, "failure"),
            (0, "probe_started\n", False, False, "failure"),
            (10, "probe_started\nprobe_errno=1\n", False, True, "failure"),
        ):
            with self.subTest(code=code, output=output, changed=changed, timeout=timeout):
                result = {"launched": True, "returncode": code, "output": output, "timeout": timeout}
                self.assertEqual(PROBE.classify_mutation(result, changed), expected)
        self.assertEqual(PROBE.classify_mutation({"launched": False}, False), "failure")

    def test_failed_nested_program_start_does_not_prove_its_write_was_blocked(self):
        result = {"launched": True, "returncode": 10, "timeout": False,
                  "output": "probe_started\nprobe_errno=1\n"}
        self.assertEqual(PROBE.classify_mutation(result, False, required_starts=2), "failure")
        result["output"] = "probe_started\nprobe_started\nprobe_errno=1\n"
        self.assertEqual(PROBE.classify_mutation(result, False, required_starts=2), "denied")


class CompareWorkloads(unittest.TestCase):
    """Runs the comparison loop against a stub invoke that fakes each host command."""

    def run_compare(self, base):
        root, binary = base / "run", base / "probe-binary"
        binary.write_bytes(b"binary")
        launcher, helper = root / "launcher", root / "helper"
        host = {"git": "/usr/bin/git", "clang": "/usr/bin/clang", "sdk": "/sdk", "ld": "/usr/bin/ld",
                "node": "/usr/bin/node", "npm": "/usr/bin/npm"}
        api = SimpleNamespace(ORIGINAL=b"original\n", CHANGED=b"changed-by-probe\n",
                              unchanged=lambda path: path.is_file() and path.read_bytes() == b"original\n",
                              invoke=self.invoke)
        with patch.object(PROBE, "bundle", return_value=(launcher, helper)), \
             patch.object(PROBE, "npm_fixture", return_value=(root, ["true"])):
            return {case["case"]: case for case in PROBE.compare(api, root, binary, "unprotected", host)["cases"]}

    @staticmethod
    def invoke(command, cwd, log, *args, **kwargs):
        if command[1:2] == ["link"]:
            os.link(command[2], command[3])
            return {"launched": True, "returncode": 10, "timeout": False,
                    "output": "probe_started\nprobe_errno=13\n"}
        if "-o" in command:
            output = Path(command[command.index("-o") + 1])
            succeeded = "/usr/bin/xcrun" in command
            if succeeded:
                output.write_bytes(b"artifact")
            return {"launched": True, "returncode": 0 if succeeded else 1, "timeout": False, "output": ""}
        program = Path(command[-1])
        ran = command[1:2] == ["exec"] and program.is_file()
        return {"launched": True, "returncode": 0 if ran else 127, "timeout": False,
                "output": "probe_started\nprobe_started\n"}

    def test_direct_build_failure_cannot_reuse_xcrun_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            cases = self.run_compare(Path(temporary))
        self.assertEqual(cases["built_program"]["observation"], "completed")
        self.assertNotEqual(cases["direct_built_program"]["observation"], "completed")
        self.assertFalse(cases["direct_built_program"]["artifact_exists"])

    def test_link_created_then_write_denied_is_not_protection(self):
        with tempfile.TemporaryDirectory() as temporary:
            cases = self.run_compare(Path(temporary))
        self.assertEqual(cases["new_hardlink_alias"]["observation"], "allowed")


if __name__ == "__main__": unittest.main()
