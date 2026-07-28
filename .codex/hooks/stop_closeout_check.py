#!/usr/bin/env python3
"""Advisory Stop hook for conflict markers introduced in tracked changes.

Experimental Codex hook; advisory only. See ADR-0024.
"""
import json
import subprocess
import sys


def conflict_diagnostics():
    try:
        out = subprocess.run(
            [
                "git",
                "-c",
                "core.whitespace=-blank-at-eol,-blank-at-eof,-space-before-tab",
                "diff",
                "--check",
                "HEAD",
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
        return [
            line
            for line in out.stdout.splitlines()
            if "leftover conflict marker" in line
        ][:10]
    except Exception:
        return []


def main() -> int:
    diagnostics = conflict_diagnostics()
    if diagnostics:
        json.dump(
            {
                "systemMessage": (
                    "Watershed closeout: merge-conflict markers found in tracked changes: "
                    + "; ".join(diagnostics)
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
