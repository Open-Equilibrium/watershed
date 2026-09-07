"""Evidence classification must not mistake a broken helper for protection."""

import importlib.util
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "app_probe", Path(__file__).resolve().parents[1] / "scripts" / "probe-macos-app-sandbox.py"
)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class NativeObservation(unittest.TestCase):
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


if __name__ == "__main__": unittest.main()
