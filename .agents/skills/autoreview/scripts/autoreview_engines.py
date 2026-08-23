from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable


class JsonStreamDisplay:
    def __init__(self, label: str, *, activity_seconds: int = 20) -> None:
        self.label = label
        self.activity_seconds = activity_seconds
        self.hidden_events = 0
        self.last_visible = time.monotonic()

    def __call__(self, name: str, line: str) -> str | None:
        if name != "stdout":
            return line
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            return self.visible(line)
        return self.json_event(event)

    def json_event(self, event: dict[str, Any]) -> str | None:
        raise NotImplementedError

    def hidden_activity(self) -> str | None:
        self.hidden_events += 1
        if time.monotonic() - self.last_visible < self.activity_seconds:
            return None
        return self.visible(self.flush_hidden())

    def flush_hidden(self) -> str:
        if not self.hidden_events:
            return ""
        count = self.hidden_events
        self.hidden_events = 0
        return f"{self.label} activity: {count} hidden tool/status events\n"

    def visible(self, text: str) -> str:
        self.last_visible = time.monotonic()
        return text


def run_with_heartbeat(
    args: list[str],
    cwd: Path,
    *,
    input_text: str | None = None,
    label: str,
    heartbeat_seconds: int = 60,
    stream_output: bool = False,
    stream_display: Callable[[str, str], str | None] | None = None,
) -> subprocess.CompletedProcess[str]:
    if stream_output:
        return run_with_stream(
            args,
            cwd,
            input_text=input_text,
            label=label,
            heartbeat_seconds=heartbeat_seconds,
            stream_display=stream_display,
        )
    started = time.monotonic()
    proc = subprocess.Popen(
        args,
        cwd=cwd,
        stdin=subprocess.PIPE if input_text is not None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    first_communicate = True
    while True:
        try:
            stdout, stderr = proc.communicate(
                input=input_text if first_communicate else None,
                timeout=heartbeat_seconds,
            )
            return subprocess.CompletedProcess(args, int(proc.returncode or 0), stdout, stderr)
        except subprocess.TimeoutExpired:
            first_communicate = False
            elapsed = int(time.monotonic() - started)
            print(f"review still running: {label} elapsed={elapsed}s pid={proc.pid}", file=sys.stderr, flush=True)


def run_with_stream(
    args: list[str],
    cwd: Path,
    *,
    input_text: str | None,
    label: str,
    heartbeat_seconds: int,
    stream_display: Callable[[str, str], str | None] | None,
) -> subprocess.CompletedProcess[str]:
    started = time.monotonic()
    proc = subprocess.Popen(
        args,
        cwd=cwd,
        stdin=subprocess.PIPE if input_text is not None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    events: queue.Queue[tuple[str, str | None]] = queue.Queue()
    stdout_parts: list[str] = []
    stderr_parts: list[str] = []

    def read_stream(name: str, stream: Any) -> None:
        try:
            for line in iter(stream.readline, ""):
                events.put((name, line))
        finally:
            events.put((name, None))

    def write_stdin() -> None:
        if proc.stdin is None or input_text is None:
            return
        try:
            proc.stdin.write(input_text)
            proc.stdin.close()
        except BrokenPipeError:
            return

    threads = [
        threading.Thread(target=read_stream, args=("stdout", proc.stdout), daemon=True),
        threading.Thread(target=read_stream, args=("stderr", proc.stderr), daemon=True),
    ]
    for thread in threads:
        thread.start()
    stdin_thread = threading.Thread(target=write_stdin, daemon=True)
    stdin_thread.start()

    open_streams = 2
    while open_streams:
        try:
            name, line = events.get(timeout=heartbeat_seconds)
        except queue.Empty:
            elapsed = int(time.monotonic() - started)
            print(f"review still running: {label} elapsed={elapsed}s pid={proc.pid}", file=sys.stderr, flush=True)
            continue
        if line is None:
            open_streams -= 1
            continue
        if name == "stdout":
            stdout_parts.append(line)
        else:
            stderr_parts.append(line)
        display = stream_display(name, line) if stream_display else line
        if display:
            target = sys.stdout if name == "stdout" else sys.stderr
            target.write(display)
            target.flush()

    for thread in threads:
        thread.join()
    stdin_thread.join(timeout=1)
    returncode = proc.wait()
    return subprocess.CompletedProcess(args, returncode, "".join(stdout_parts), "".join(stderr_parts))


def resolve_command(name: str, repo: Path) -> str:
    resolved = find_command(name, repo)
    if resolved:
        return resolved
    raise SystemExit(f"executable not found: {name}. Install it or pass an explicit trusted path when supported.")


def find_command(name: str, repo: Path) -> str | None:
    command = Path(name)
    if has_directory_component(name, command):
        base = command if command.is_absolute() else repo / command
        return first_executable_candidate(base)
    for part in os.environ.get("PATH", "").split(os.pathsep):
        if not part or part == ".":
            continue
        path_part = Path(part)
        if not path_part.is_absolute():
            continue
        try:
            resolved_part = path_part.resolve()
            resolved_repo = repo.resolve()
        except OSError:
            continue
        if is_within(resolved_part, resolved_repo):
            continue
        found = first_executable_candidate(resolved_part / name, reject_root=resolved_repo)
        if found:
            return found
    return None


def is_within(path: Path, root: Path) -> bool:
    return path == root or path.is_relative_to(root)


def has_directory_component(name: str, command: Path) -> bool:
    separators = [separator for separator in (os.sep, os.altsep) if separator]
    return command.is_absolute() or bool(command.drive) or any(separator in name for separator in separators)


def first_executable_candidate(path: Path, *, reject_root: Path | None = None) -> str | None:
    if os.name == "nt" and not path.suffix:
        extensions = [ext for ext in os.environ.get("PATHEXT", ".COM;.EXE;.BAT;.CMD").split(";") if ext]
        candidates = [path.with_suffix(ext.lower()) for ext in extensions]
        candidates.extend(path.with_suffix(ext.upper()) for ext in extensions)
        candidates.append(path)
    else:
        candidates = [path]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            if reject_root is not None:
                try:
                    if is_within(candidate.resolve(), reject_root):
                        continue
                except OSError:
                    continue
            return str(candidate)
    return None
