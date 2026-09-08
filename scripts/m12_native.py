"""Native installer/readiness acceptance helpers (CI only, never installed)."""

import ctypes
import json
import os
import pathlib
import platform as host_platform
import shutil
import signal
import subprocess
import sys
import tempfile


def linux_fault_filter(fault):
    # x86_64 only. These inherited, process-local filters inject EPERM without
    # changing kernel settings, files, services or the host's security policy.
    load_number = (0x20, 0, 0, 0)
    load_argument = (0x20, 0, 0, 16)
    deny = (0x06, 0, 0, 0x50001)
    allow = (0x06, 0, 0, 0x7FFF0000)
    if fault == "user-namespace":
        return [load_number, (0x15, 0, 1, 435), (0x06, 0, 0, 0x50026),
                (0x15, 2, 0, 272), (0x15, 1, 0, 56), allow,
                load_argument, (0x45, 0, 1, 0x10000000), deny, allow]
    if fault == "seccomp":
        return [load_number, (0x15, 0, 1, 317), deny,
                (0x15, 0, 3, 157), load_argument, (0x15, 0, 1, 22), deny, allow]
    raise ValueError(f"unknown fault: {fault}")


def validate_probe(output, *, ready, platform, backend):
    probe = json.loads(output)
    assert set(probe) == {"backend", "backend_version", "executor", "executor_version",
                          "platform", "protocol_versions", "ready", "schema",
                          "supported_policy_features"}, probe
    assert probe["executor"] == "flow-executor", probe
    assert probe["schema"] == "flow-executor-probe-v0", probe
    assert probe["protocol_versions"] == ["0"], probe
    assert probe["supported_policy_features"] == (
        ["flow-owned-write-protection"] if ready else []), probe
    assert probe["ready"] is ready, probe
    assert probe["platform"] == platform, probe
    assert probe["backend"] == backend, probe
    return probe


def inject_linux_fault(fault):
    class Filter(ctypes.Structure):
        _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte),
                    ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint)]

    class Program(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ushort), ("filter", ctypes.POINTER(Filter))]

    instructions = linux_fault_filter(fault)
    filters = (Filter * len(instructions))(*(Filter(*item) for item in instructions))
    program = Program(len(filters), filters)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(38, 1, 0, 0, 0) or libc.prctl(22, 2, ctypes.byref(program), 0, 0):
        raise OSError(ctypes.get_errno(), "could not install process-local readiness fault")


def run(command, *, env=None, fault=None, timeout=20, capture=True):
    before_exec = None
    if fault and sys.platform == "linux":
        before_exec = lambda: inject_linux_fault(fault)
    elif fault and sys.platform == "darwin":
        # The outer profile denies only the nested launch, not the whole host.
        command = ["/usr/bin/sandbox-exec", "-p",
                   '(version 1)(allow default)'
                   '(deny process-exec (literal "/usr/bin/sandbox-exec"))', *command]
    with subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE if capture else None,
                          stderr=subprocess.PIPE if capture else None,
                          start_new_session=True, preexec_fn=before_exec) as child:
        try:
            stdout, stderr = child.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.communicate(timeout=2)
            raise AssertionError(f"native acceptance exceeded {timeout}s: {command[0]}")
        return subprocess.CompletedProcess(command, child.returncode, stdout, stderr)


def native_host():
    identity = (sys.platform, host_platform.machine())
    if identity == ("linux", "x86_64"):
        return "ubuntu-24.04-x86_64", "bubblewrap-seccomp", ("user-namespace", "seccomp")
    if identity == ("darwin", "arm64"):
        return "macos-26-aarch64", "seatbelt", ("seatbelt-launch",)
    raise AssertionError(f"native acceptance requires a supported native host: {identity}")


def readiness_negatives():
    platform, backend, faults = native_host()
    assert os.geteuid() != 0, "run readiness acceptance as the existing unprivileged user"
    repository = pathlib.Path(__file__).resolve().parents[1]
    artifacts = pathlib.Path(os.environ.get(
        "M12_COVERAGE_BIN_DIR", repository / "target/m12-standard/release")).resolve()
    with tempfile.TemporaryDirectory(prefix="m12-readiness-", dir=os.environ["RUNNER_TEMP"]) as temporary:
        root = pathlib.Path(temporary).resolve()
        bundle = root / "bundle"
        bundle.mkdir(mode=0o755)
        for name, source in (("install.sh", repository / "install/install.sh"),
                             ("flow", artifacts / "flow"),
                             ("flow-executor", artifacts / "flow-executor")):
            shutil.copyfile(source, bundle / name)
            (bundle / name).chmod(0o755)
        home = root / "home"
        home.mkdir(mode=0o700)
        config = home / "Library/Application Support" if sys.platform == "darwin" else root / "config"
        config.mkdir(mode=0o700, parents=True)
        environment = {"PATH": "", "HOME": str(home), "XDG_CONFIG_HOME": str(config)}
        if "LLVM_PROFILE_FILE" in os.environ:
            environment["LLVM_PROFILE_FILE"] = os.environ["LLVM_PROFILE_FILE"]
        baseline = run([str(bundle / "flow-executor"), "--probe"], env=environment)
        assert baseline.returncode == 0, baseline.stderr
        validate_probe(baseline.stdout, ready=True, platform=platform, backend=backend)
        assert baseline.stderr == b"", baseline.stderr
        for fault in faults:
            # Use the actual native Executor, not a probe stub or changed OS policy.
            probe = run([str(bundle / "flow-executor"), "--probe"], env=environment, fault=fault)
            assert probe.returncode == 0, probe.stderr
            validate_probe(probe.stdout, ready=False, platform=platform, backend=backend)
            assert probe.stderr.startswith(b"flow-executor readiness: "), probe.stderr
            assert len(probe.stderr) <= 1024, probe.stderr
            checked = run([str(bundle / "flow"), "executor", "check"], env=environment, fault=fault)
            assert checked.returncode == 65, checked.stderr
            assert checked.stdout == b"", checked.stdout
            assert checked.stderr.startswith(b"error: executor_unavailable:"), checked.stderr
            prefix = root / fault
            installed = run(["/bin/sh", str(bundle / "install.sh"), "--prefix", str(prefix)],
                            env=environment, fault=fault)
            assert installed.returncode == 1, installed.stderr
            assert installed.stdout == b"", installed.stdout
            assert b"executor_unavailable:" in installed.stderr, installed.stderr
            assert b"--no-default-executor" in installed.stderr, installed.stderr
            assert list((prefix / "bin").iterdir()) == [], "failed installation did not roll back"
            opted_out = run(["/bin/sh", str(bundle / "install.sh"), "--prefix", str(prefix),
                             "--no-default-executor"], env=environment, fault=fault)
            assert opted_out.returncode == 0, opted_out.stderr
            assert sorted(path.name for path in (prefix / "bin").iterdir()) == ["flow"]
            assert not (config / "flow-agent/executor.json").exists()
            print(f"native readiness negative passed: {fault}", flush=True)
        restored = run([str(bundle / "flow-executor"), "--probe"], env=environment)
        assert restored.returncode == 0, restored.stderr
        validate_probe(restored.stdout, ready=True, platform=platform, backend=backend)
        assert restored.stderr == b"", restored.stderr


def main():
    if sys.argv[1:] == ["readiness"]:
        readiness_negatives()
    elif len(sys.argv) == 3 and sys.argv[1] == "acceptance":
        native_host()
        assert os.geteuid() != 0, "run installer acceptance as the existing unprivileged user"
        result = run(["/bin/sh", sys.argv[2], "--bounded"], timeout=600, capture=False)
        raise SystemExit(result.returncode)
    else:
        raise SystemExit("usage: m12_native.py readiness | acceptance <script>")


if __name__ == "__main__":
    main()
