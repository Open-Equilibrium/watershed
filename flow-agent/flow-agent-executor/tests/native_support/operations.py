"""Finite file/device operations retained from macos-self-protection-cases.py."""
import ctypes
import json
import mmap
import os
import sys
import time
from pathlib import Path


def xattr(path, value=None):
    name = b"user.flow-test"
    if sys.platform != "darwin":
        if value is not None:
            os.setxattr(path, name, value)
        return os.getxattr(path, name).decode()
    libc = ctypes.CDLL(None, use_errno=True)
    if value is not None:
        result = libc.setxattr(os.fsencode(path), name, value, len(value), 0, 0)
    else:
        buffer = ctypes.create_string_buffer(32)
        result = libc.getxattr(os.fsencode(path), name, buffer, len(buffer), 0, 0)
    if result < 0:
        raise OSError(ctypes.get_errno(), "native xattr")
    return value.decode() if value is not None else buffer.raw[:result].decode()


config = json.loads(Path("operations.json").read_text())
Path("helper-ready").write_text("ready")
if config["wait"]:
    deadline = time.monotonic() + 10
    while not Path("release").exists():
        if time.monotonic() >= deadline:
            raise TimeoutError("controller did not release helper")
        time.sleep(.005)
results = []
for row in config["operations"]:
    action, target = row["action"], row["target"]
    number = 0
    try:
        if action == "write":
            Path(target).write_bytes(b"changed")
        elif action == "chmod":
            os.chmod(target, 0o777)
        elif action == "xattr":
            xattr(target, b"changed")
        elif action == "mmap":
            with open(target, "r+b") as file:
                with mmap.mmap(file.fileno(), 0) as mapping:
                    mapping[0] = ord("X")
                    mapping.flush()
        elif action == "link":
            os.link(target, row["auxiliary"])
        elif action == "fd-write":
            assert os.write(int(target), b"changed") == 7
        elif action == "fd-create":
            fd = os.open("from-handle", os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                         0o600, dir_fd=int(target))
            os.close(fd)
        elif action == "fd-stat":
            os.fstat(int(target))
        elif action == "terminal":
            if row.get("indirect"):
                target = Path(target).read_text()
            fd = os.open(target, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
            os.close(fd)
        else:
            raise ValueError(action)
    except OSError as error:
        number = error.errno
    result = {"case": row["case"], "errno": number}
    if action == "xattr":
        try:
            result["value"] = xattr(target)
        except OSError as error:
            # ENODATA on Linux, ENOATTR on Darwin: absence, not unreadability.
            assert error.errno == (93 if sys.platform == "darwin" else 61), error
            result["value"] = None
    results.append(result)
Path("operations-result.json").write_text(json.dumps(results))
