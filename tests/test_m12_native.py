"""Portable process-boundary tests; these do not certify a native backend."""

import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, call, patch, sentinel

from scripts import m12_native as native


class NativeProcessTest(unittest.TestCase):
    def setUp(self):
        self.popen = self.enterContext(patch.object(native.subprocess, "Popen"))
        self.child = self.popen.return_value.__enter__.return_value
        self.child.pid = 123
        self.child.returncode = 0
        self.child.communicate.return_value = (b"output", b"diagnostic")
        self.boundary = Mock()
        self.boundary.attach_mock(self.child.communicate, "communicate")
        self.enterContext(patch.object(native, "os", SimpleNamespace(
            killpg=self.boundary.killpg, geteuid=lambda: 1000)))
        self.enterContext(patch.object(native, "signal", SimpleNamespace(
            SIGTERM=sentinel.term, SIGKILL=sentinel.kill)))

    def test_result_streams_and_launch_options(self):
        for code, options, timeout, capture in (
            (0, {}, 20, True),
            (7, {"timeout": 5, "capture": False}, 5, False),
        ):
            with self.subTest(code=code):
                self.boundary.reset_mock()
                self.child.returncode = code
                streams = (b"output", b"diagnostic") if capture else (None, None)
                self.child.communicate.return_value = streams
                result = native.run(["fixture"], env={"FIXTURE": "1"}, **options)
                self.assertEqual((result.args, result.returncode, result.stdout, result.stderr),
                                 (["fixture"], code, *streams))
                self.popen.assert_called_with(
                    ["fixture"], env={"FIXTURE": "1"}, stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE if capture else None,
                    stderr=subprocess.PIPE if capture else None,
                    start_new_session=True, preexec_fn=None)
                self.assertEqual(self.boundary.mock_calls, [call.communicate(timeout=timeout)])

    def test_timeout_cleanup_is_ordered_and_bounded(self):
        for timeouts in (1, 2, 3):
            with self.subTest(timeouts=timeouts):
                self.boundary.reset_mock()
                expired = subprocess.TimeoutExpired("fixture", 2)
                self.child.communicate.side_effect = [expired] * timeouts + [(b"", b"")]
                expected_error = subprocess.TimeoutExpired if timeouts == 3 else AssertionError
                with self.assertRaises(expected_error):
                    native.run(["fixture"])
                expected = [call.communicate(timeout=20), call.killpg(123, sentinel.term),
                            call.communicate(timeout=2)]
                if timeouts > 1:
                    expected += [call.killpg(123, sentinel.kill), call.communicate(timeout=2)]
                self.assertEqual(self.boundary.mock_calls, expected)

    def test_process_errors_propagate(self):
        for stage in ("launch", "communicate", "signal"):
            with self.subTest(stage=stage):
                self.popen.side_effect = None
                self.child.communicate.side_effect = None
                self.boundary.killpg.side_effect = None
                error = OSError("fixture failure")
                if stage == "launch":
                    self.popen.side_effect = error
                elif stage == "communicate":
                    self.child.communicate.side_effect = error
                else:
                    self.child.communicate.side_effect = subprocess.TimeoutExpired("fixture", 20)
                    self.boundary.killpg.side_effect = error
                with self.assertRaises(OSError) as raised:
                    native.run(["fixture"])
                self.assertIs(raised.exception, error)

    def test_acceptance_uses_outer_deadline_and_propagates_exit(self):
        self.enterContext(patch.object(native, "sys", SimpleNamespace(
            argv=["m12_native.py", "acceptance", "fixture.sh"])))
        host = self.enterContext(patch.object(native, "native_host"))
        for code in (0, 7):
            with self.subTest(code=code):
                self.child.returncode = code
                with self.assertRaises(SystemExit) as raised:
                    native.main()
                self.assertEqual(raised.exception.code, code)
                self.child.communicate.assert_called_with(timeout=600)
                self.popen.assert_called_with(
                    ["/bin/sh", "fixture.sh", "--bounded"], env=None,
                    stdin=subprocess.DEVNULL, stdout=None, stderr=None,
                    start_new_session=True, preexec_fn=None)
        self.assertEqual(host.call_count, 2)


if __name__ == "__main__":
    unittest.main()
