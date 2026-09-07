#!/usr/bin/env python3
"""Dev-only native comparison; local ad-hoc signatures, synthetic files, no product backend."""

import errno
import importlib.util
import json
import os
import platform
import plistlib
import select
import shutil
import sys
import tempfile
import time
from pathlib import Path


SOURCE = r'''
#import <Foundation/Foundation.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
static const char value[] = "changed-by-probe\n";
static int failed(void) {
    int number = errno;
    printf("probe_errno=%d\n", number);
    return number == EACCES || number == EPERM ? 10 : 11;
}
static int write_path(const char *path) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) return failed();
    if (write(fd, value, sizeof(value)-1) != (ssize_t)(sizeof(value)-1)) { close(fd); return 12; }
    return close(fd) == 0 ? 0 : 12;
}
int main(int argc, char **argv) { @autoreleasepool {
    if (argc < 3) return 70;
    puts("probe_started"); fflush(stdout);
    const char *action = argv[1], *target = argv[2];
    if (!strcmp(action, "write")) return write_path(target);
    if (!strcmp(action, "terminal")) {
        int fd = open(target, O_WRONLY | O_NOCTTY); if (fd < 0) return failed();
        int code = write(fd, value, sizeof(value)-1) == (ssize_t)(sizeof(value)-1) ? 0 : failed();
        close(fd); return code;
    }
    if (!strcmp(action, "unlink")) return unlink(target) == 0 ? 0 : failed();
    if (!strcmp(action, "chmod")) return chmod(target, 0400) == 0 ? 0 : failed();
    if (!strcmp(action, "replace") && argc == 4) return rename(argv[3], target) == 0 ? 0 : failed();
    if (!strcmp(action, "rename") && argc == 4) return rename(target, argv[3]) == 0 ? 0 : failed();
    if (!strcmp(action, "link") && argc == 4) return link(target, argv[3]) == 0 ? write_path(argv[3]) : failed();
    if (!strcmp(action, "fd")) {
        int fd = atoi(target);
        return pwrite(fd, value, sizeof(value)-1, 0) == (ssize_t)(sizeof(value)-1) ? 0 : failed();
    }
    if (!strcmp(action, "mmap")) {
        int fd = open(target, O_RDWR); if (fd < 0) return failed();
        void *memory = mmap(NULL, sizeof(value)-1, PROT_WRITE|PROT_READ, MAP_SHARED, fd, 0);
        if (memory == MAP_FAILED) { int code = failed(); close(fd); return code; }
        memcpy(memory, value, sizeof(value)-1);
        int code = msync(memory, sizeof(value)-1, MS_SYNC) == 0 ? 0 : failed();
        munmap(memory, sizeof(value)-1); close(fd); return code;
    }
    if (!strcmp(action, "child")) {
        pid_t child = fork(); if (child < 0) return failed();
        if (!child) { if (setsid() < 0) exit(failed()); exit(write_path(target)); }
        int status; if (waitpid(child, &status, 0) != child) return failed();
        return WIFEXITED(status) ? WEXITSTATUS(status) : 71;
    }
    if (!strcmp(action, "exec")) { execv(target, &argv[2]); return failed(); }
    if (!strcmp(action, "container")) {
        NSString *home = NSHomeDirectory();
        BOOL isolated = [home containsString:@"/Library/Containers/org.watershed.probe."];
        printf("container_isolated=%d\n", isolated);
        NSString *path = [home stringByAppendingPathComponent:[NSString stringWithUTF8String:target]];
        return isolated ? write_path(path.fileSystemRepresentation) : 72;
    }
    return 70;
} }
'''


def classify_mutation(result, changed, required_starts=1):
    """A launch failure, unrelated error, timeout or missing effect is never protection evidence."""
    if not result.get("launched") or result.get("timeout") or result.get("output", "").splitlines().count("probe_started") < required_starts:
        return "failure"
    if result.get("returncode") == 0 and changed:
        return "allowed"
    numbers = {f"probe_errno={errno.EPERM}", f"probe_errno={errno.EACCES}"}
    if result.get("returncode") == 10 and not changed and numbers.intersection(result["output"].splitlines()):
        return "denied"
    return "failure"


def require(api, command, root, name):
    result = api.invoke(command, root, root / f"{name}.log", timeout_seconds=30, file_limit=4*1024*1024)
    if not result.get("launched") or result.get("timeout") or result.get("returncode") != 0:
        raise RuntimeError(json.dumps({"setup": name, "command": result}, separators=(",", ":")))
    return result


def bundle(api, root, binary, sandboxed, grant):
    identifier = "org.watershed.probe." + root.parent.name.replace("-", ".") + "." + root.name
    app = root / "Probe.app"
    contents = app / "Contents"; executables = contents / "MacOS"
    executables.mkdir(parents=True)
    launcher, helper = executables / "launcher", executables / "helper"
    for destination in (launcher, helper): shutil.copyfile(binary, destination); destination.chmod(0o700)
    with (contents / "Info.plist").open("wb") as output:
        plistlib.dump({"CFBundleIdentifier": identifier, "CFBundleExecutable": "launcher",
                      "CFBundlePackageType": "APPL", "CFBundleVersion": "1", "LSBackgroundOnly": True}, output)
    for name, target, inherited in (("helper", helper, True), ("launcher", app, False)):
        entitlements = {}
        if sandboxed:
            entitlements["com.apple.security.app-sandbox"] = True
            if inherited: entitlements["com.apple.security.inherit"] = True
            else:
                entitlements["com.apple.security.files.user-selected.read-write"] = True
                if grant:
                    entitlements["com.apple.security.temporary-exception.files.absolute-path.read-write"] = [str(grant) + "/"]
        path = root / f"{name}.entitlements"
        with path.open("wb") as output: plistlib.dump(entitlements, output)
        require(api, ["/usr/bin/codesign", "--force", "--sign", "-", "--options", "runtime", "--identifier",
                      identifier + (".helper" if inherited else ""), "--entitlements", str(path), str(target)], root, name + "-sign")
    require(api, ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], root, "verify")
    return launcher, helper


def compare(api, root, binary, mode, host):
    root.mkdir(); project = root / "project"; project.mkdir()
    protected = root / "flow-parent" / "flow-owned"; protected.mkdir(parents=True)
    nested = project / "flow-owned"; nested.mkdir()
    sandboxed = mode.startswith("app-")
    launcher, helper = bundle(api, root, binary, sandboxed, project if mode == "app-project-exception" else None)
    external = project / "unmodified-program"; shutil.copyfile(binary, external); external.chmod(0o700)
    prefix = [str(launcher)]
    if mode == "profile-guard":
        policy = root / "guard.sb"
        parameters = {}
        policy.write_text(api.profile(protected, True, additional_roots=[nested], read_denied=[Path("/dev/tty")],
                                      parameters=parameters), encoding="utf-8")
        prefix = [str(api.SANDBOX_EXEC), *[part for key, value in parameters.items() for part in ("-D", f"{key}={value}")],
                  "-f", str(policy), *prefix]
    results = []
    for name, action in (("project_write", "write"), ("protected_write", "write"), ("protected_delete", "unlink"),
                         ("atomic_replace", "replace"), ("metadata_write", "chmod"), ("mapped_write", "mmap"),
                         ("new_child_session", "child"), ("bundled_helper", "helper"),
                         ("unmodified_program", "external"), ("symlink_alias", "symlink"),
                         ("existing_hardlink_alias", "hardlink"), ("new_hardlink_alias", "link"),
                         ("inherited_writable_handle", "fd"), ("nested_protected_write", "nested"),
                         ("protected_parent_move", "rename")):
        target = (project if name == "project_write" else nested if action == "nested" else protected) / f"{name}.bin"
        target.write_bytes(api.ORIGINAL); target.chmod(0o600)
        auxiliary = project / f"{name}-aux.bin"
        command, handle, fds = [action, str(target)], None, ()
        if action in {"symlink", "hardlink"}:
            if action == "symlink": auxiliary.symlink_to(target)
            else: os.link(target, auxiliary)
            command = ["write", str(auxiliary)]
        elif action == "replace": auxiliary.write_bytes(api.CHANGED); command.append(str(auxiliary))
        elif action == "link": command.append(str(auxiliary))
        elif action == "nested": command[0] = "write"
        elif action in {"helper", "external"}: command = ["exec", str(helper if action == "helper" else external), "write", str(target)]
        elif action == "rename": command = ["rename", str(protected.parent), str(auxiliary)]
        elif action == "fd":
            handle = target.open("r+b", buffering=0); fds = (handle.fileno(),); command = ["fd", str(handle.fileno())]
        if mode == "profile-guard" and action == "hardlink":
            results.append({"case": name, "observation": "readiness_rejected", "protected_intact": api.unchanged(target)})
            auxiliary.unlink(); continue
        result = api.invoke([*prefix, *command], project, root / f"{name}.log", () if mode == "profile-guard" else fds)
        if handle: handle.close()
        changed = not api.unchanged(target) or target.stat().st_mode & 0o777 != 0o600
        nested_start = action in {"helper", "external"}
        observation = classify_mutation(result, changed, required_starts=2 if nested_start else 1)
        if nested_start and result.get("output", "").splitlines().count("probe_started") < 2:
            observation = "program_not_started"
        if mode == "profile-guard" and action == "fd" and result.get("returncode") == 11 and "probe_errno=9" in result.get("output", "") and not changed:
            observation = "closed_handle"
        results.append({"case": name, "observation": observation, "changed": changed, "command": result})
        if action == "rename" and auxiliary.exists(): auxiliary.rename(protected.parent)
        if action in {"symlink", "hardlink", "link"} and auxiliary.exists(): auxiliary.unlink()
    master, slave = os.openpty()
    try:
        result = api.invoke([*prefix, "terminal", os.ttyname(slave)], project, root / "terminal.log")
        delivered = bool(select.select([master], [], [], 0)[0])
        if delivered: delivered = os.read(master, 256).replace(b"\r\n", b"\n") == api.CHANGED
        results.append({"case": "controller_terminal_device", "observation": classify_mutation(result, delivered),
                        "changed": delivered, "command": result})
    finally:
        os.close(master); os.close(slave)
    source = project / "hello.c"; source.write_text('int main(void) { return 0; }\n', encoding="utf-8")
    built = project / "built-program"
    workloads = [
        ("system_shell", ["exec", "/bin/sh", "-c", "printf 'host_shell_ran\\n'"]),
        ("system_git", ["exec", "/usr/bin/git", "--version"]),
        ("host_compiler_version", ["exec", "/usr/bin/xcrun", "clang", "--version"]),
        ("project_program", ["exec", str(external), "write", str(project / "program-output")]),
        ("host_build", ["exec", "/usr/bin/xcrun", "clang", "-arch", "arm64", str(source), "-o", str(built)]),
        ("built_program", ["exec", str(built)]),
        ("direct_git", ["exec", host["git"], "--version"]),
        ("direct_git_project", ["exec", host["git"], "-c", "init.defaultBranch=probe", "init", "--quiet", str(project / "synthetic-repo")]),
        ("direct_host_build", ["exec", host["clang"], "-arch", "arm64", "-isysroot", host["sdk"],
                               "-B", str(Path(host["ld"]).parent), str(source), "-o", str(built)]),
        ("direct_built_program", ["exec", str(built)]),
    ]
    if sandboxed: workloads.append(("own_container", ["container", "synthetic-probe.bin"]))
    for name, command in workloads:
        started = time.perf_counter_ns()
        result = api.invoke([*prefix, *command], project, root / f"{name}.log", timeout_seconds=30, file_limit=4*1024*1024)
        elapsed = time.perf_counter_ns() - started
        results.append({"case": name, "observation": "completed" if result.get("returncode") == 0 and not result.get("timeout") else "not_completed",
                        "command": result, "elapsed_ns": elapsed,
                        "artifact_exists": built.is_file() if name in {"host_build", "built_program", "direct_host_build", "direct_built_program"}
                            else (project / "synthetic-repo" / ".git").is_dir() if name == "direct_git_project"
                            else (project / "program-output").is_file() if name == "project_program" else None})
    return {"mode": mode, "cases": results}


def main():
    evidence = {"experiment": "app-sandbox-comparison", "os": platform.system(), "arch": platform.machine(),
                "macos_version": platform.mac_ver()[0], "kernel_release": platform.release()}
    if platform.system() != "Darwin" or platform.machine().lower() not in {"arm64", "aarch64"}:
        evidence["error"] = "Native macOS ARM64 required; no probe executed."
        print(json.dumps(evidence)); return 2
    spec = importlib.util.spec_from_file_location("profile_probe", Path(__file__).with_name("probe-macos-self-protection.py"))
    api = importlib.util.module_from_spec(spec); spec.loader.exec_module(api)
    api.CHILD_ENV.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"})
    try:
        # Only this unique synthetic directory is created/read/removed; no existing home data is inspected.
        with tempfile.TemporaryDirectory(prefix="watershed-app-probe-", dir=Path.home()) as temporary:
            root = Path(temporary).resolve()
            source = root / "probe.m"; source.write_text(SOURCE, encoding="utf-8")
            binary = root / "original-helper"
            evidence["compiler"] = require(api, ["/usr/bin/xcrun", "clang", "--version"], root, "compiler")
            require(api, ["/usr/bin/xcrun", "clang", "-arch", "arm64", "-Wall", "-Wextra", "-Werror", "-framework", "Foundation",
                          str(source), "-o", str(binary)], root, "compile")
            host = {name: require(api, ["/usr/bin/xcrun", "--find", name], root, "find-" + name)["output"].strip()
                    for name in ("git", "clang", "ld")}
            host["sdk"] = require(api, ["/usr/bin/xcrun", "--show-sdk-path"], root, "find-sdk")["output"].strip()
            evidence["host_tool_paths"] = host
            evidence["results"] = []
            for mode in ("unprotected", "profile-guard", "app-minimal", "app-project-exception"):
                result = compare(api, root / mode, binary, mode, host)
                evidence["results"].append(result)
                print(json.dumps({"experiment": evidence["experiment"], **result}, separators=(",", ":")), flush=True)
    except (OSError, ValueError, RuntimeError) as error:
        evidence["error"] = str(error)
        print(json.dumps(evidence, separators=(",", ":"))); return 2
    rows = [row for result in evidence["results"] for row in result["cases"]]
    evidence["measurement_complete"] = not any(row["observation"] == "failure" for row in rows)
    del evidence["results"]
    print(json.dumps(evidence, separators=(",", ":")))
    return 0 if evidence["measurement_complete"] else 2


if __name__ == "__main__": raise SystemExit(main())
