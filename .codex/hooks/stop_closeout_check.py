#!/usr/bin/env python3
"""Stop hook: a fast, ADVISORY closeout check. Warns (never blocks, never loops)
if the working tree still has leftover merge-conflict markers in tracked files
when a turn ends — cheap insurance against shipping conflict cruft. Output-quality
only; it makes no continuation decision.

Experimental Codex hook; advisory only. See ADR-0024.
"""
import json
from pathlib import Path
import subprocess
import sys


def repo_root():
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, timeout=10,
        )
        return Path(out.stdout.strip() or ".")
    except Exception:
        return Path(".")


def changed_files(root):
    try:
        out = subprocess.run(
            ["git", "diff", "--name-only", "HEAD"],
            cwd=root,
            capture_output=True, text=True, timeout=10,
        )
        return [f for f in out.stdout.splitlines() if f.strip()]
    except Exception:
        return []


def has_conflict(root, path):
    start = end = False
    try:
        with (root / path).open("r", encoding="utf-8", errors="ignore") as fh:
            for line in fh:
                if line.startswith("<<<<<<< "):
                    start = True
                elif line.startswith(">>>>>>> "):
                    end = True
                if start and end:
                    return True
    except Exception:
        return False
    return False


def main() -> int:
    root = repo_root()
    flagged = [p for p in changed_files(root)[:200] if has_conflict(root, p)]
    if flagged:
        json.dump(
            {
                "systemMessage": (
                    "Watershed closeout: merge-conflict markers found in "
                    + ", ".join(flagged[:10])
                    + " — resolve before running the PR closeout."
                )
            },
            sys.stdout,
        )
    else:
        json.dump({}, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
