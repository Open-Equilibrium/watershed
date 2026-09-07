#!/usr/bin/env python3
"""Dev-only macOS sandbox-exec feasibility probe; never a runtime backend."""

import argparse
import errno
import json
import os
import platform
import signal
import subprocess
import sys
import tempfile
from pathlib import Path


ORIGINAL, CHANGED = b"protected-fixture-v1\n", b"changed-by-probe\n"
TIMEOUT, OUTPUT_LIMIT, MARKER_LIMIT = 5, 64 * 1024, 256
SANDBOX_EXEC = Path("/usr/bin/sandbox-exec")
CHILD_ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C", "PYTHONDONTWRITEBYTECODE": "1"}

PYTHON_HELPER = r"""
import json, os, subprocess, sys
from pathlib import Path
action, target, auxiliary, started, result = sys.argv[1:]
Path(started).write_bytes(b"started")
def record(state, number=None):
    payload = {"state": state}
    if number is not None: payload["errno"] = number
    Path(result).write_bytes(json.dumps(payload, separators=(",", ":")).encode()[:256])
try:
    if action == "write": Path(target).write_bytes(b"changed-by-probe\n")
    elif action == "unlink": Path(target).unlink()
    elif action == "replace":
        Path(auxiliary).write_bytes(b"changed-by-probe\n"); os.replace(auxiliary, target)
    elif action == "link":
        os.link(target, auxiliary); Path(auxiliary).write_bytes(b"changed-by-probe\n")
    elif action == "relocate":
        ancestor = Path(target).parent.parent
        ancestor.rename(auxiliary)
        (Path(auxiliary) / Path(target).relative_to(ancestor)).write_bytes(b"changed-by-probe\n")
    elif action == "fd":
        fd = int(target); os.lseek(fd, 0, os.SEEK_SET); os.write(fd, b"changed-by-probe\n"); os.fsync(fd)
    elif action == "spawn":
        nested = result + ".nested"
        code = r'''import json,sys
from pathlib import Path
try: Path(sys.argv[1]).write_bytes(b'changed-by-probe\n')
except OSError as e: Path(sys.argv[2]).write_text(json.dumps({'state':'error','errno':e.errno})); raise SystemExit(10)
Path(sys.argv[2]).write_text('{"state":"ok"}')'''
        child = subprocess.run([sys.executable, "-c", code, target, nested], stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3)
        nested_result = json.loads(Path(nested).read_text(encoding="utf-8"))
        if child.returncode == 10 and nested_result.get("state") == "error":
            record("error", nested_result.get("errno")); raise SystemExit(10)
        if child.returncode != 0 or nested_result.get("state") != "ok":
            raise RuntimeError("nested helper failed")
    else: raise ValueError("unknown action")
except OSError as error:
    record("error", error.errno); raise SystemExit(10)
else:
    record("ok")
"""

BUILD_WORKLOAD = r"""
export TMPDIR="$PWD"
/usr/bin/xcode-select -p
/usr/bin/xcrun --find clang
/usr/bin/xcrun clang --version
/usr/bin/xcrun clang -arch arm64 -x c -o "$1" - <<'WATERSHED_PROBE_SOURCE'
#include <errno.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc != 2) return 70;
    FILE *output = fopen(argv[1], "wb");
    if (!output) return errno == EACCES || errno == EPERM ? 10 : 71;
    const char value[] = "changed-by-probe\n";
    if (fwrite(value, 1, sizeof(value) - 1, output) != sizeof(value) - 1) return 72;
    return fclose(output) == 0 ? 0 : 73;
}
WATERSHED_PROBE_SOURCE
/usr/bin/codesign --verify --strict --verbose=2 "$1"
"$1" "$2"
write_status=0
"$1" "$3" || write_status=$?
printf 'protected_write_exit=%s\n' "$write_status"
test "$write_status" -eq "$4"
"""


def profile(protected_root: Path, protect_ancestors: bool = False,
            additional_roots=(), files=(), read_denied=(), parameters=None) -> str:
    def quote(path):
        if parameters is None: return json.dumps(str(path), ensure_ascii=False)
        key = f"PATH_{len(parameters)}"; parameters[key] = str(path)
        return f'(param "{key}")'
    roots = [protected_root, *additional_roots]
    policy = "(version 1)\n(allow default)\n"
    for root in roots: policy += "(deny file-write* (subpath %s))\n" % quote(root)
    for path in files: policy += "(deny file-write* (literal %s))\n" % quote(path)
    for path in read_denied: policy += "(deny file-read* file-write* (literal %s))\n" % quote(path)
    if protect_ancestors:
        ancestors = {ancestor for path in [*roots, *files] for ancestor in path.parents}
        for ancestor in sorted(ancestors):
            policy += "(deny file-write-unlink (literal %s))\n" % quote(ancestor)
    return policy


def limit_output(file_limit: int) -> None:
    import resource  # macOS-only call site
    resource.setrlimit(resource.RLIMIT_FSIZE, (file_limit, file_limit))


def invoke(command: list[str], cwd: Path, log: Path, fds: tuple[int, ...] = (),
           timeout_seconds: int = TIMEOUT, file_limit: int = OUTPUT_LIMIT) -> dict:
    """No stdin, no inherited environment, bounded wait, and OS-enforced output cap."""
    try:
        with log.open("wb") as output:
            child = subprocess.Popen(command, cwd=cwd, env=CHILD_ENV, stdin=subprocess.DEVNULL,
                stdout=output, stderr=subprocess.STDOUT, close_fds=True, pass_fds=fds,
                start_new_session=True, preexec_fn=lambda: limit_output(file_limit))
            try: code, timeout = child.wait(timeout=timeout_seconds), False
            except subprocess.TimeoutExpired:
                try: os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError: pass
                code, timeout = child.wait(timeout=2), True
    except OSError as error:
        return {"launched": False, "error": type(error).__name__}
    with log.open("rb") as captured:
        text = captured.read(OUTPUT_LIMIT).decode("utf-8", "replace").strip()
    return {"launched": True, "returncode": code, "timeout": timeout, "output": text[:2048]}


def fixture(root: Path, name: str) -> dict:
    case_root = root / name
    protected, scratch, outside = case_root / "protected", case_root / "scratch", case_root / "outside"
    for directory in (protected, scratch, outside): directory.mkdir(parents=True)
    file = protected / "record.bin"; file.write_bytes(ORIGINAL)
    return {"root": case_root, "protected": protected, "file": file, "scratch": scratch, "outside": outside}


def read_marker(path: Path) -> dict | None:
    try:
        raw = path.read_bytes()
        value = json.loads(raw) if len(raw) <= MARKER_LIMIT else None
        return value if isinstance(value, dict) and value.get("state") in {"ok", "error"} else None
    except (OSError, ValueError): return None


def unchanged(path: Path) -> bool:
    return path.is_file() and path.read_bytes() == ORIGINAL


def run_case(mode: str, root: Path, name: str, kind: str) -> dict:
    data, handle, fds = fixture(root, name), None, ()
    target, auxiliary, denied = data["file"], data["scratch"] / "staged.bin", True
    if kind == "scratch": kind, target, denied = "write", data["scratch"] / "allowed.bin", False
    elif kind == "symlink":
        target = data["outside"] / "symlink-alias"; target.symlink_to(data["file"]); kind = "write"
    elif kind == "hardlink":
        target = data["outside"] / "hardlink-alias"; os.link(data["file"], target); kind = "write"
    elif kind == "link": auxiliary = data["outside"] / "new-hardlink-alias"
    elif kind == "relocate":
        container = data["root"] / "container"; container.mkdir()
        data["protected"] = data["protected"].rename(container / "protected")
        target = data["file"] = data["protected"] / "record.bin"
        auxiliary = data["outside"] / "relocated-container"
    elif kind == "handle":
        handle = data["file"].open("r+b", buffering=0); target, kind, fds = Path(str(handle.fileno())), "fd", (handle.fileno(),)
    if mode == "guarded-launch" and data["file"].stat().st_nlink != 1:
        intact = unchanged(data["file"])
        return {"case": name, "outcome": "pass" if name == "hardlink_alias" and intact else "failure",
                "started": False, "readiness_rejection": "existing_hardlink", "protected_intact": intact}
    started, marker = data["scratch"] / "started.marker", data["scratch"] / "result.json"
    command = [sys.executable, "-c", PYTHON_HELPER, kind, str(target), str(auxiliary), str(started), str(marker)]
    if mode != "baseline":
        policy = data["root"] / "probe.sb"
        policy.write_text(profile(data["protected"], mode == "guarded-launch"), encoding="utf-8")
        command = [str(SANDBOX_EXEC), "-f", str(policy), *command]
    result = invoke(command, data["root"], data["scratch"] / "child.log", () if mode == "guarded-launch" else fds)
    if handle is not None: handle.close()
    helper_started = started.is_file() and started.read_bytes() == b"started"
    child, intact = read_marker(marker), unchanged(data["file"])
    permitted = not denied and target.is_file() and target.read_bytes() == CHANGED
    explicit_denial = child and child.get("state") == "error" and child.get("errno") in {errno.EACCES, errno.EPERM}
    closed_handle = mode == "guarded-launch" and kind == "fd" and child and child.get("state") == "error" and child.get("errno") == errno.EBADF
    succeeded = child and child.get("state") == "ok" and result.get("returncode") == 0
    if not result.get("launched") or result.get("timeout") or not helper_started or child is None:
        outcome = "failure"
    elif mode == "baseline":
        if not succeeded:
            outcome = "failure"
        elif denied:
            outcome = "red_control" if not intact else "failure"
        else:
            outcome = "pass" if permitted else "failure"
    elif not denied:
        outcome = "pass" if succeeded and permitted else "violation" if explicit_denial else "failure"
    elif explicit_denial or closed_handle:
        outcome = "pass" if result["returncode"] == 10 and intact else "violation"
    elif succeeded:
        outcome = "violation"
    else: outcome = "failure"
    return {"case": name, "outcome": outcome, "started": helper_started, "returncode": result.get("returncode"),
            "child": child, "protected_intact": intact, "permitted_control": permitted, "closed_handle": bool(closed_handle),
            "detail": result.get("error") or result.get("output", "")}


def safe_case(*args) -> dict:
    try: return run_case(*args)
    except (OSError, ValueError) as error: return {"case": args[-2], "outcome": "failure", "detail": type(error).__name__}


def run_build_case(root: Path, guarded: bool) -> dict:
    name = "guarded_host_build" if guarded else "unprotected_host_build"
    data = fixture(root, name)
    binary, allowed = data["scratch"] / "native-writer", data["scratch"] / "project.bin"
    command = ["/bin/sh", "-eu", "-c", BUILD_WORKLOAD, "probe", str(binary), str(allowed),
               str(data["file"]), "10" if guarded else "0"]
    if guarded:
        policy = data["root"] / "probe.sb"
        policy.write_text(profile(data["protected"], True), encoding="utf-8")
        command = [str(SANDBOX_EXEC), "-f", str(policy), *command]
    result = invoke(command, data["scratch"], data["scratch"] / "build.log",
                    timeout_seconds=30, file_limit=4 * 1024 * 1024)
    permitted = allowed.is_file() and allowed.read_bytes() == CHANGED
    protected_ok = unchanged(data["file"]) if guarded else data["file"].read_bytes() == CHANGED
    passed = result.get("launched") and not result.get("timeout") and result.get("returncode") == 0 and permitted and protected_ok
    return {"case": name, "outcome": "pass" if passed else "failure", "command": result,
            "project_write": permitted, "protected_outcome": protected_ok,
            "binary_bytes": binary.stat().st_size if binary.is_file() else None}


def emit(evidence: dict, code: int) -> int:
    print(json.dumps(evidence, separators=(",", ":"))); return code


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=("baseline", "profile", "guarded-launch", "host-build",
                                                         "lifecycle-baseline", "lifecycle"))
    args = parser.parse_args(); system, machine = platform.system(), platform.machine().lower()
    evidence = {"mode": args.mode, "os": system, "arch": machine, "macos_version": platform.mac_ver()[0], "kernel_release": platform.release()}
    if system != "Darwin" or machine not in {"arm64", "aarch64"}:
        evidence["error"] = "This experimental probe requires macOS ARM64; syntax-check only elsewhere."
        return emit(evidence, 2)
    if not SANDBOX_EXEC.is_file() or not os.access(SANDBOX_EXEC, os.X_OK):
        evidence["error"] = "/usr/bin/sandbox-exec is unavailable; no native probe was run."
        return emit(evidence, 2)
    cases = [("scratch_write", "scratch"), ("protected_write", "write"), ("protected_delete", "unlink"),
             ("atomic_replace", "replace"), ("child_inherits_guard", "spawn"), ("symlink_alias", "symlink"),
             ("hardlink_alias", "hardlink"), ("inherited_writable_handle", "handle"),
             ("new_hardlink_alias", "link"), ("protected_ancestor_relocation", "relocate")]
    with tempfile.TemporaryDirectory(prefix="watershed-sandbox-probe-") as temporary:
        root = Path(temporary).resolve()
        if args.mode.startswith("lifecycle"):
            import runpy
            cases_module = runpy.run_path(str(Path(__file__).with_name("macos-self-protection-cases.py")))
            evidence["cases"] = cases_module["run"](sys.modules[__name__], root, args.mode == "lifecycle")
        elif args.mode == "host-build":
            try:
                evidence["cases"] = [run_build_case(root, guarded) for guarded in (False, True)]
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                evidence["error"] = type(error).__name__
                return emit(evidence, 2)
        else:
            evidence["cases"] = [safe_case(args.mode, root, *case) for case in cases]
    outcomes = [case["outcome"] for case in evidence["cases"]]
    if args.mode == "baseline":
        evidence["red_control_complete"] = outcomes == ["pass"] + ["red_control"] * (len(outcomes) - 1)
        return emit(evidence, 1 if evidence["red_control_complete"] else 2)
    evidence["ok"] = all(outcome == "pass" for outcome in outcomes)
    return emit(evidence, 0 if evidence["ok"] else 2 if "failure" in outcomes else 1)


if __name__ == "__main__": raise SystemExit(main())
