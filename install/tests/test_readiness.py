import array
import contextlib
import json
import os
import pathlib
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest

from scripts.m12_native import linux_fault_filter, validate_probe


class _UnreapedExit:
    def __init__(self, child):
        self.child = child
        self.seen = False
        self.queue = select.kqueue() if sys.platform == "darwin" else None
        if self.queue is not None:
            try:
                self.queue.control([select.kevent(child.pid, filter=select.KQ_FILTER_PROC,
                    flags=select.KQ_EV_ADD | select.KQ_EV_ENABLE | select.KQ_EV_ONESHOT,
                    fflags=select.KQ_NOTE_EXIT)], 0, 0)
            except BaseException:
                self.queue.close()
                raise

    def exited(self):
        if self.child.returncode is not None:
            return True
        if self.queue is not None:
            self.seen |= bool(self.queue.control(None, 1, 0))
        else:
            self.seen |= os.waitid(os.P_PID, self.child.pid,
                os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
        return self.seen

    def close(self):
        if self.queue is not None:
            self.queue.close()


def _terminate_owned_group(child, notice=None):
    # Only the original, still-unreaped start_new_session child owns this group.
    if child.returncode is not None:
        raise AssertionError("fixture group was already reaped")
    primary = None
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError as error:
        # Darwin excludes zombies from group signaling and returns EPERM when
        # none remain. These fixture groups cannot change UID; keep live errors.
        if sys.platform != "darwin" or notice is None or not notice.exited():
            primary = error
    except BaseException as error:
        primary = error
    try:
        child.wait(timeout=2)
    except BaseException as error:
        _cleanup_errors(primary, [error])
    if primary is not None:
        raise primary


def _cleanup_errors(primary, errors):
    if not errors:
        return
    if primary is None:
        primary = errors.pop(0)
        for error in errors:
            primary.add_note(f"fixture teardown: {error!r}")
        raise primary
    for error in errors:
        primary.add_note(f"fixture teardown: {error!r}")


def _capture_timeout_note(error, child, notice):
    try:
        error.add_note(f"root_exit_observed_without_reaping={notice.exited()}")
    except Exception as diagnostic_error:
        error.add_note(f"root exit observation failed: {diagnostic_error!r}")
    for name in ("stdout", "stderr"):
        stream = getattr(child, name)
        tail = getattr(error, "output" if name == "stdout" else "stderr") or b""
        residual = bytearray()
        state = "closed"
        try:
            if stream is not None and not stream.closed:
                os.set_blocking(stream.fileno(), False)
                state = "64KiB cap; EOF inconclusive"
                while len(residual) < 65536:
                    try:
                        chunk = os.read(stream.fileno(), min(4096, 65536 - len(residual)))
                    except BlockingIOError:
                        state = "EAGAIN; writer remains"
                        break
                    if not chunk:
                        state = "EOF"
                        break
                    residual.extend(chunk)
        except Exception as diagnostic_error:
            state = f"snapshot failed: {diagnostic_error!r}"
        error.add_note(f"{name}: {state}; tail:\n"
                       + (tail + residual)[-16384:].decode("utf-8", errors="replace"))


@contextlib.contextmanager
def _captured_readiness(command, **options):
    child = subprocess.Popen(command, start_new_session=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, **options)
    notice = None
    try:
        notice = _UnreapedExit(child)
        yield child, notice
    finally:
        primary, errors = sys.exc_info()[1], []
        try:
            if child.returncode is None:
                _terminate_owned_group(child, notice)
        except BaseException as error:
            errors.append(error)
        for stream in (child.stdin, child.stdout, child.stderr):
            if stream is not None:
                try:
                    stream.close()
                except BaseException as error:
                    errors.append(error)
        if notice is not None:
            try:
                notice.close()
            except BaseException as error:
                errors.append(error)
        _cleanup_errors(primary, errors)


def _receive_inner(server, deadline):
    # Keep the real inner argv/control/capture descriptors, but own its sole wait.
    server.settimeout(max(0, deadline - time.monotonic()))
    peer, _ = server.accept()
    descriptors = array.array("i")
    try:
        peer.settimeout(max(0, deadline - time.monotonic()))
        request, ancillary, flags, _ = peer.recvmsg(65536,
            socket.CMSG_SPACE(3 * descriptors.itemsize))
        for level, kind, payload in ancillary:
            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                descriptors.frombytes(payload[:len(payload) // descriptors.itemsize * descriptors.itemsize])
        if flags & socket.MSG_CTRUNC or len(descriptors) != 3:
            raise AssertionError("inner fixture must transfer exactly three descriptors")
        while not request.endswith(b"\n"):
            peer.settimeout(max(0, deadline - time.monotonic()))
            extra = peer.recv(65536 - len(request))
            if not extra:
                raise AssertionError("incomplete inner fixture request")
            request += extra
        child = subprocess.Popen(json.loads(request), start_new_session=True,
            stdin=descriptors[0], stdout=descriptors[1], stderr=descriptors[2], env={"PATH": ""})
        try:
            peer.sendall(b"S")
        except BaseException as primary:
            try:
                _terminate_owned_group(child)
            except BaseException as error:
                primary.add_note(f"fixture teardown: {error!r}")
            raise
        return child, peer
    except BaseException as primary:
        try:
            peer.close()
        except BaseException as error:
            primary.add_note(f"fixture teardown: {error!r}")
        raise
    finally:
        for descriptor in descriptors:
            os.close(descriptor)


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
                    "import array, json, pathlib, signal, socket, subprocess, sys, time\n"
                    "phase, separate, marker, owner = sys.argv[1:5]\n"
                    "if phase == 'failed': sys.exit(65)\n"
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    "if phase == 'after-inner':\n"
                    "    if separate == 'True':\n"
                    "        channel = socket.socket(socket.AF_UNIX)\n"
                    "        channel.settimeout(10)\n"
                    "        channel.connect(owner)\n"
                    "        request = (json.dumps(sys.argv[5:]) + '\\n').encode()\n"
                    "        sent = channel.sendmsg([request], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', [0, 1, 2]))])\n"
                    "        channel.sendall(request[sent:])\n"
                    "        assert channel.recv(1) == b'S'\n"
                    "    else: child = subprocess.Popen(sys.argv[5:])\n"
                    "pathlib.Path(marker).touch()\n"
                    "if phase == 'after-inner':\n"
                    "    if separate == 'True':\n"
                    "        try: channel.recv(1)\n"
                    "        except socket.timeout: pass\n"
                    "        sys.exit(0)\n"
                    "    child.wait()\n"
                    "time.sleep(10)\n"
                )
                owner = root / "inner-owner"
                with contextlib.ExitStack() as resources:
                    server = resources.enter_context(socket.socket(socket.AF_UNIX)) if separate_group else None
                    if server is not None:
                        server.bind(str(owner))
                        server.listen(1)
                    with _captured_readiness(
                        ["/bin/sh", "-c", body, "flow-readiness", "outer", str(home), str(flow),
                         str(home / "status"), str(channel), str(os.getpgrp()), scanner, body,
                         sys.executable, "-c", helper, phase, str(separate_group), str(ready), str(owner)],
                        stdin=subprocess.PIPE, env={"PATH": ""},
                    ) as (process, notice):
                        self.assertNotEqual(process.pid, os.getpgrp())
                        inner = peer = inner_notice = None
                        try:
                            process.stdin.write(b"start\n")
                            process.stdin.flush()
                            admission_deadline = time.monotonic() + 2
                            if server is not None:
                                inner, peer = _receive_inner(server, admission_deadline)
                                inner_notice = _UnreapedExit(inner)
                            if phase != "failed":
                                while time.monotonic() < admission_deadline:
                                    if ready.exists() and (phase != "after-inner" or (home / "status").exists()):
                                        break
                                    self.assertFalse(notice.exited(), "outer exited before helper admission")
                                    time.sleep(0.01)
                                else:
                                    self.fail("helper did not reach its cleanup boundary")
                            started = time.monotonic()
                            process.stdin.close()
                            process.stdin = None
                            try:
                                _, stderr = process.communicate(timeout=5)
                            except subprocess.TimeoutExpired as error:
                                _capture_timeout_note(error, process, notice)
                                raise
                            self.assertLess(time.monotonic() - started, 5, stderr)
                            self.assertIn(process.returncode, (0, -signal.SIGKILL), stderr)
                            if phase == "after-inner":
                                self.assertEqual((home / "status").read_text(), "65\n")
                            else:
                                self.assertFalse((home / "status").exists())
                        finally:
                            primary, errors = sys.exc_info()[1], []
                            if inner is not None:
                                try:
                                    _terminate_owned_group(inner, inner_notice)
                                except BaseException as error:
                                    errors.append(error)
                            if peer is not None:
                                try:
                                    peer.close()  # The helper receives our owned shutdown EOF.
                                except BaseException as error:
                                    errors.append(error)
                            if inner_notice is not None:
                                try:
                                    inner_notice.close()
                                except BaseException as error:
                                    errors.append(error)
                            _cleanup_errors(primary, errors)

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
