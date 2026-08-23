from __future__ import annotations

import argparse
import tempfile
from pathlib import Path

import autoreview_engines as engines

def run_droid(args: argparse.Namespace, repo: Path, prompt: str) -> str:
    if args.thinking:
        raise SystemExit("--thinking is not supported by the droid engine")
    if args.tools:
        raise SystemExit(
            "droid requires --no-tools because its tool-enabled mode has no enforced read-only sandbox"
        )
    prompt_path = Path(tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False).name)
    try:
        prompt_path.write_text(prompt, encoding="utf-8")
        cmd = [
            engines.resolve_command(args.droid_bin, repo),
            "exec",
            "--cwd",
            str(repo),
            "--output-format",
            "json",
            "-f",
            str(prompt_path),
        ]
        if args.model:
            cmd.extend(["--model", args.model])
        cmd.extend(["--disabled-tools", "*"])
        result = engines.run_with_heartbeat(
            cmd,
            repo,
            label="droid",
            stream_output=args.stream_engine_output,
        )
        if result.returncode != 0:
            raise SystemExit(f"droid engine failed ({result.returncode})\n{result.stderr or result.stdout}")
        return result.stdout
    finally:
        prompt_path.unlink(missing_ok=True)
