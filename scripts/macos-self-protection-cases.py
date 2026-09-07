"""Dev-only protected-object lifecycle cases; not a production scanner or launcher."""

import errno
import json
import os
import stat
import subprocess
import sys
import time
from pathlib import Path


def admit(roots: list[Path], files: list[Path], limit: int = 4096) -> int:
    """Bounded fixture scan under exclusive fixture ownership, not race-safe admission."""
    names, identities = set(), {}

    def visit(path: Path, depth: int) -> None:
        if path in names: return
        names.add(path)
        if len(names) > limit or depth > 32: raise ValueError("inventory limit")
        info = path.lstat()
        if stat.S_ISDIR(info.st_mode):
            for child in path.iterdir(): visit(child, depth + 1)
        elif stat.S_ISREG(info.st_mode):
            key = info.st_dev, info.st_ino
            count, links = identities.get(key, (0, info.st_nlink))
            if links != info.st_nlink: raise ValueError("inventory changed")
            identities[key] = count + 1, links
        else: raise ValueError("unsupported protected object")

    for root in [*roots, *files]: visit(root, 0)
    if any(count != links for count, links in identities.values()):
        raise ValueError("external hardlink")
    return sum(count for count, _ in identities.values())


OPERATIONS = r"""
import errno, json, mmap, os, subprocess, sys, time
from pathlib import Path
plan, result, ready, release = map(Path, sys.argv[1:])
rows = json.loads(plan.read_text())
ready.write_bytes(b'ready')
deadline = time.monotonic() + 5
while not release.exists():
    if time.monotonic() >= deadline: raise SystemExit(72)
    time.sleep(.01)
observations = []
for row in rows:
    action, target, auxiliary = row['action'], Path(row['target']), row.get('auxiliary', '')
    try:
        if action == 'write': target.write_bytes(b'changed-by-probe\n')
        elif action == 'create': target.mkdir()
        elif action == 'unlink': target.unlink()
        elif action == 'replace':
            Path(auxiliary).write_bytes(b'changed-by-probe\n'); os.replace(auxiliary, target)
        elif action == 'link': os.link(target, auxiliary)
        elif action == 'rename': target.rename(auxiliary); Path(auxiliary).rename(target)
        elif action == 'chmod': target.chmod(0o777)
        elif action == 'xattr': os.setxattr(target, 'user.watershed-probe', b'changed')
        elif action == 'mmap':
            with target.open('r+b') as opened:
                with mmap.mmap(opened.fileno(), 0) as mapped: mapped[0:1] = b'X'; mapped.flush()
        elif action == 'terminal':
            fd = os.open(target, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK); os.close(fd)
        elif action == 'child':
            code = 'import os,sys; os.setsid(); open(sys.argv[1], "wb").write(b"changed-by-probe\\n")'
            child = subprocess.run([sys.executable, '-c', code, str(target)],
                stdin=subprocess.DEVNULL, capture_output=True, timeout=3)
            observations.append({'case': row['case'], 'returncode': child.returncode,
                'denied': child.returncode == 1 and b'PermissionError' in child.stderr})
            continue
        else: raise ValueError('unknown operation')
    except OSError as error:
        observations.append({'case': row['case'], 'errno': error.errno,
                             'denied': error.errno in (errno.EPERM, errno.EACCES)})
    else: observations.append({'case': row['case'], 'denied': False, 'returncode': 0})
result.write_text(json.dumps(observations))
"""


def lifecycle(probe, root: Path, guarded: bool) -> list[dict]:
    import pty  # Native call site only; the admission tests also run on Windows.

    homes = [root / "owner" / "homes" / name for name in ("first", "second")]
    store, project, bin_dir = root / "platform-store", root / "project", root / "bin"
    for directory in [*homes, store, project, bin_dir]: directory.mkdir(parents=True)
    files = [bin_dir / name for name in ("flow", "flow-executor", "custom-executor")]
    roots = [*homes, store]
    protected = [homes[0] / name for name in ("config", "registry", "runtime")]
    protected += [homes[1] / "config", store / "credentials", store / "executor", *files]
    for path in protected: path.write_bytes(probe.ORIGINAL)
    rows, expected = [], {}

    def operation(name, action, target, auxiliary=None, denied=True):
        row = {"case": name, "action": action, "target": str(target)}
        if auxiliary is not None: row["auxiliary"] = str(auxiliary)
        rows.append(row); expected[name] = denied

    for index, path in enumerate(protected): operation(f"protected_object_{index}", "write", path)
    operation("project_write", "write", project / "allowed", denied=False)
    operation("project_hardlink", "link", project / "allowed", project / "alias", denied=False)
    internal = homes[0] / "internal-stage"; os.link(protected[0], internal)
    operation("internal_publication_alias", "write", internal)
    operations = ("unlink", "replace", "chmod", "xattr", "mmap", "link", "child")
    for action in operations:
        target = homes[0] / ("operation-" + action); target.write_bytes(probe.ORIGINAL)
        protected.append(target)
        operation(action, action, target, project / (action + "-auxiliary"))
    operation("new_directory", "create", homes[0] / "tool-created")
    operation("ancestor_relocation", "rename", homes[0].parent, project / "relocated")
    late = homes[0] / "published-after-launch"
    stage = homes[0] / "late-stage"
    operation("late_hardlink_publication", "write", late)
    operation("unfinished_internal_stage", "write", stage)
    replacement = homes[0] / "replacement"; replacement.write_bytes(probe.ORIGINAL)
    operation("controller_replacement", "write", replacement)
    directory = homes[0] / "published-directory"
    operation("controller_directory_publication", "write", directory / "record")
    master, slave = pty.openpty()
    terminal = Path(os.ttyname(slave)).resolve()
    operation("review_terminal_handle", "terminal", terminal)
    plan, result, ready, release = [project / name for name in ("plan.json", "result.json", "ready", "release")]
    plan.write_text(json.dumps(rows), encoding="utf-8")
    inventory_before = admit(roots, files)
    command = [sys.executable, "-c", OPERATIONS, str(plan), str(result), str(ready), str(release)]
    if guarded:
        policy = project / "policy.sb"
        policy.write_text(probe.profile(homes[0], True, roots[1:], files, [terminal]), encoding="utf-8")
        command = [str(probe.SANDBOX_EXEC), "-f", str(policy), *command]
    child = None
    try:
        with (project / "child.log").open("wb") as log:
            child = subprocess.Popen(command, cwd=project, env=probe.CHILD_ENV, stdin=subprocess.DEVNULL,
                stdout=log, stderr=subprocess.STDOUT, close_fds=True, start_new_session=True,
                preexec_fn=lambda: probe.limit_output(probe.OUTPUT_LIMIT))
            deadline = time.monotonic() + 5
            while not ready.exists() and child.poll() is None and time.monotonic() < deadline: time.sleep(.01)
            if not ready.is_file(): raise ValueError("native helper did not become ready")
            # Real trusted-parent operations after the protected child has started.
            stage.write_bytes(probe.ORIGINAL); os.link(stage, late)
            staged_replacement = homes[0] / "replace-stage"; staged_replacement.write_bytes(probe.ORIGINAL)
            os.replace(staged_replacement, replacement)
            staging_dir = homes[0] / "directory-stage"; staging_dir.mkdir()
            (staging_dir / "record").write_bytes(probe.ORIGINAL); staging_dir.rename(directory)
            with (homes[0] / "runtime").open("ab") as stream: stream.write(b"controller-append\n")
            protected.remove(homes[0] / "runtime")
            inventory_after = admit(roots, files)
            release.write_bytes(b"go")
            code = child.wait(timeout=10)
        if code != 0 or result.stat().st_size > probe.OUTPUT_LIMIT: raise ValueError("native helper failed")
        observations = json.loads(result.read_text())
        if [row["case"] for row in observations] != list(expected): raise ValueError("incomplete observations")
        for row in observations:
            correct = row["denied"] == (guarded and expected[row["case"]])
            row["outcome"] = "pass" if correct else "violation"
        intact = all(probe.unchanged(path) for path in [*protected, internal, late, stage, replacement, directory / "record"])
        runtime_intact = (homes[0] / "runtime").read_bytes() == probe.ORIGINAL + b"controller-append\n"
        observations.append({"case": "controller_publication_integrity", "outcome": "pass" if
            (intact and runtime_intact if guarded else not intact) else "violation",
            "inventory_before": inventory_before, "inventory_after": inventory_after})
        return observations
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, 9); child.wait(timeout=2)
        os.close(slave); os.close(master)


def run(probe, root: Path, guarded: bool) -> list[dict]:
    try: return lifecycle(probe, root, guarded)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return [{"case": "lifecycle", "outcome": "failure", "detail": str(error)}]
