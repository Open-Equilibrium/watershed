#!/usr/bin/env python3
"""Dev-only macOS sandbox-exec feasibility probe; never a runtime backend."""

import argparse
import errno
import json
import os
import platform
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


def profile(protected_root: Path) -> str:
    literal = json.dumps(str(protected_root), ensure_ascii=False)
    return "(version 1)\n(allow default)\n(deny file-write* (subpath %s))\n" % literal


def limit_output() -> None:
    import resource  # macOS-only call site
    resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_LIMIT, OUTPUT_LIMIT))


def invoke(command: list[str], cwd: Path, log: Path, fds: tuple[int, ...] = ()) -> dict:
    """No stdin, no inherited environment, bounded wait, and OS-enforced output cap."""
    try:
        with log.open("wb") as output:
            child = subprocess.Popen(command, cwd=cwd, env=CHILD_ENV, stdin=subprocess.DEVNULL,
                stdout=output, stderr=subprocess.STDOUT, close_fds=True, pass_fds=fds,
                preexec_fn=limit_output)
            try: code, timeout = child.wait(timeout=TIMEOUT), False
            except subprocess.TimeoutExpired:
                child.kill(); code, timeout = child.wait(timeout=2), True
    except OSError as error:
        return {"launched": False, "error": type(error).__name__}
    text = log.read_bytes()[:OUTPUT_LIMIT].decode("utf-8", "replace").strip()
    return {"launched": True, "returncode": code, "timeout": timeout, "output": text[:512]}


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
    started, marker = data["scratch"] / "started.marker", data["scratch"] / "result.json"
    command = [sys.executable, "-c", PYTHON_HELPER, kind, str(target), str(auxiliary), str(started), str(marker)]
    if mode == "profile":
        policy = data["root"] / "probe.sb"; policy.write_text(profile(data["protected"]), encoding="utf-8")
        command = [str(SANDBOX_EXEC), "-f", str(policy), *command]
    result = invoke(command, data["root"], data["scratch"] / "child.log", fds)
    if handle is not None: handle.close()
    helper_started = started.is_file() and started.read_bytes() == b"started"
    child, intact = read_marker(marker), unchanged(data["file"])
    permitted = not denied and target.is_file() and target.read_bytes() == CHANGED
    explicit_denial = child and child.get("state") == "error" and child.get("errno") in {errno.EACCES, errno.EPERM}
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
    elif explicit_denial:
        outcome = "pass" if result["returncode"] == 10 and intact else "violation"
    elif succeeded:
        outcome = "violation"
    else: outcome = "failure"
    return {"case": name, "outcome": outcome, "started": helper_started, "returncode": result.get("returncode"),
            "child": child, "protected_intact": intact, "permitted_control": permitted,
            "detail": result.get("error") or result.get("output", "")}


def safe_case(*args) -> dict:
    try: return run_case(*args)
    except (OSError, ValueError) as error: return {"case": args[-2], "outcome": "failure", "detail": type(error).__name__}


def emit(evidence: dict, code: int) -> int:
    print(json.dumps(evidence, separators=(",", ":"))); return code


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument("--mode", required=True, choices=("baseline", "profile"))
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
        evidence["cases"] = [safe_case(args.mode, root, *case) for case in cases]
    outcomes = [case["outcome"] for case in evidence["cases"]]
    if args.mode == "baseline":
        evidence["red_control_complete"] = outcomes == ["pass"] + ["red_control"] * (len(outcomes) - 1)
        return emit(evidence, 1 if evidence["red_control_complete"] else 2)
    evidence["ok"] = all(outcome == "pass" for outcome in outcomes)
    return emit(evidence, 0 if evidence["ok"] else 2 if "failure" in outcomes else 1)


if __name__ == "__main__": raise SystemExit(main())
