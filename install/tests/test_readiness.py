import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest

from scripts.m12_native import linux_fault_filter, validate_probe


def filter_result(program, syscall, argument=0):
    accumulator = 0
    index = 0
    while index < len(program):
        code, yes, no, value = program[index]
        if code == 0x20:
            accumulator = {0: syscall, 16: argument}[value]
        elif code == 0x15:
            index += yes if accumulator == value else no
        elif code == 0x45:
            index += yes if accumulator & value else no
        elif code == 0x06:
            return value
        else:
            raise AssertionError(f"unexpected BPF instruction: {code}")
        index += 1
    raise AssertionError("fault filter did not return a decision")


class ReadinessContractTest(unittest.TestCase):
    def privileged_installer_acceptance(self):
        # Explicit CI invocation only; ordinary discovery remains unprivileged.
        import errno
        import pwd
        import pty
        import select
        import traceback

        from install.tests.test_install import PrefixInstallerTest

        self.assertIn(sys.platform, ("linux", "darwin"))
        self.assertEqual(os.geteuid(), 0, "requires root and an explicit non-root SUDO_USER")
        user = pwd.getpwnam(os.environ["SUDO_USER"])
        self.assertNotEqual(user.pw_uid, 0)
        helper = "/usr/bin/sudo" if sys.platform == "darwin" else "/usr/sbin/runuser"
        self.assertTrue(os.access(helper, os.X_OK), helper)
        child, terminal = pty.fork()
        if child == 0:
            try:
                case = PrefixInstallerTest()
                case.readiness_user = user
                case.test_standard_and_opt_out_install_from_any_cwd_with_empty_path()
                case.test_failed_readiness_rolls_back_every_published_artifact()
                case.test_signal_during_readiness_terminates_descendants_and_rolls_back()
            except BaseException:
                traceback.print_exc()
                sys.stderr.flush()
                os._exit(1)
            sys.stdout.flush()
            os._exit(0)
        output = bytearray()
        status = None
        terminal_open = True
        deadline = time.monotonic() + 60
        try:
            while time.monotonic() < deadline:
                readable, _, _ = select.select([terminal] if terminal_open else [], [], [], 0.05)
                if readable:
                    try:
                        chunk = os.read(terminal, 4096)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        chunk = b""
                    if not chunk:
                        terminal_open = False
                    else:
                        output.extend(chunk)
                    self.assertLessEqual(len(output), 65536, "privileged fixture output overflow")
                if status is None:
                    reaped, result = os.waitpid(child, os.WNOHANG)
                    if reaped:
                        status = result
                if status is not None and not terminal_open:
                    break
            else:
                self.fail("privileged fixture exceeded 60 seconds")
            text = output.decode("utf-8", errors="replace")
            print(text, end="", flush=True)
            self.assertEqual(os.waitstatus_to_exitcode(status), 0, text)
        finally:
            os.close(terminal)
            if status is None:
                # The original child stays ours until this sole waitpid reaps it.
                try:
                    os.kill(child, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                os.waitpid(child, 0)

    def test_namespace_fault_denies_namespace_creation_not_ordinary_children(self):
        program = linux_fault_filter("user-namespace")
        for syscall, argument in ((272, 0x10000000), (56, 0x10000011)):
            self.assertEqual(filter_result(program, syscall, argument), 0x50001)
        self.assertEqual(filter_result(program, 435), 0x50026)  # clone3 -> ENOSYS
        for syscall, argument in ((56, 17), (59, 0), (157, 22), (317, 1)):
            self.assertEqual(filter_result(program, syscall, argument), 0x7FFF0000)

    def test_seccomp_fault_denies_filter_installation_not_process_startup(self):
        program = linux_fault_filter("seccomp")
        for syscall, argument in ((157, 22), (317, 1)):
            self.assertEqual(filter_result(program, syscall, argument), 0x50001)
        for syscall, argument in ((157, 38), (56, 17), (59, 0), (272, 0x10000000)):
            self.assertEqual(filter_result(program, syscall, argument), 0x7FFF0000)

    def test_outer_supervisor_bounds_failed_and_stalled_helpers(self):
        source = (pathlib.Path(__file__).parents[1] / "install.sh").read_text()
        body = source.partition("readiness_shell='\n")[2].partition("\n'\nwait_for_readiness_status()")[0]
        self.assertTrue(body)
        cases = (("failed", False, "/usr/bin/pgrep"),
                 ("before-inner", False, "/usr/bin/pgrep"),
                 ("after-inner", False, "/usr/bin/pgrep"),
                 ("after-inner", True, "/usr/bin/pgrep"),
                 ("after-inner", True, "/missing/pgrep"))
        for phase, separate_group, scanner in cases:
            with self.subTest(phase=phase, separate_group=separate_group, scanner=scanner), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                home = root / "home"
                home.mkdir(mode=0o700)
                channel = root / "control"
                os.mkfifo(channel, 0o600)
                ready = root / "helper-ready"
                flow = root / "flow"
                flow.write_text("#!/bin/sh\nexit 65\n")
                flow.chmod(0o755)
                helper = (
                    "import pathlib, signal, subprocess, sys, time\n"
                    "phase, separate, marker = sys.argv[1:4]\n"
                    "if phase == 'failed': sys.exit(65)\n"
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    "if phase == 'after-inner':\n"
                    "    child = subprocess.Popen(sys.argv[4:], start_new_session=separate == 'True')\n"
                    "pathlib.Path(marker).touch()\n"
                    "if phase == 'after-inner': child.wait()\n"
                    "time.sleep(10)\n"
                )
                process = subprocess.Popen(
                    ["/bin/sh", "-c", body, "flow-readiness", "outer", str(home), str(flow),
                     str(home / "status"), str(channel), str(os.getpgrp()), scanner, body,
                     sys.executable, "-c", helper, phase, str(separate_group), str(ready)],
                    start_new_session=True, stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, env={"PATH": ""},
                )
                self.assertNotEqual(process.pid, os.getpgrp())
                try:
                    process.stdin.write(b"start\n")
                    process.stdin.flush()
                    if phase != "failed":
                        for _ in range(200):
                            if ready.exists() and (phase != "after-inner" or (home / "status").exists()):
                                break
                            self.assertIsNone(process.poll(), "outer exited before helper admission")
                            time.sleep(0.01)
                        else:
                            self.fail("helper did not reach its cleanup boundary")
                    started = time.monotonic()
                    process.stdin.close()
                    process.stdin = None
                    _, stderr = process.communicate(timeout=5)
                    self.assertLess(time.monotonic() - started, 5, stderr)
                    self.assertIn(process.returncode, (0, -signal.SIGKILL), stderr)
                    if phase == "after-inner":
                        self.assertEqual((home / "status").read_text(), "65\n")
                    else:
                        self.assertFalse((home / "status").exists())
                finally:
                    if process.stdin is not None:
                        process.stdin.close()
                        process.stdin = None
                    if process.poll() is None:
                        process.kill()
                        process.communicate(timeout=5)

    def test_probe_has_only_retained_native_metadata(self):
        probe = dict(backend="seatbelt", backend_version="27.0",
                     executor="flow-executor", executor_version="0.0.0",
                     platform="macos-27-aarch64", protocol_versions=["0"], ready=True,
                     schema="flow-executor-probe-v0",
                     supported_policy_features=["flow-owned-write-protection"])
        def validate(value, ready=True):
            return validate_probe(json.dumps(value), ready=ready,
                                  platform="macos-27-aarch64", backend="seatbelt")
        self.assertEqual(validate(probe), probe)
        for key, value in (("runtime_mounts", []), ("unknown", True),
                           ("supported_policy_features", ["static-self-reexec"]),
                           ("protocol_versions", ["1"]), ("executor", "other")):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                validate({**probe, key: value})
        unavailable = {**probe, "ready": False, "supported_policy_features": []}
        self.assertEqual(validate(unavailable, ready=False), unavailable)
        with self.assertRaises(AssertionError):
            validate(unavailable)


if __name__ == "__main__":
    unittest.main()
