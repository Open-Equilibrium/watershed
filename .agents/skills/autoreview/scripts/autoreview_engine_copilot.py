from __future__ import annotations

import argparse
import os
import tempfile
from pathlib import Path

import autoreview_engines as engines

def run_copilot(args: argparse.Namespace, repo: Path, prompt: str) -> str:
    if args.thinking:
        raise SystemExit("--thinking is not supported by the copilot engine")
    if not args.tools:
        raise SystemExit("--no-tools is not supported by the copilot engine; copilot requires a read-only file view tool to load the review bundle without exposing it in argv")
    with tempfile.TemporaryDirectory(prefix="autoreview-copilot.") as tempdir:
        prompt_path = Path(tempdir) / "prompt.txt"
        prompt_path.write_text(prompt, encoding="utf-8")
        os.chmod(prompt_path, 0o600)
        cmd = [
            engines.resolve_command(args.copilot_bin, repo),
            "-C",
            tempdir,
            "-p",
            "Read ./prompt.txt and follow it exactly. Return only the requested JSON object.",
            "--output-format",
            "json",
            "--stream",
            "on" if args.stream_engine_output else "off",
            "--no-ask-user",
            "--disable-builtin-mcps",
        ]
        if args.model:
            cmd.extend(["--model", args.model])
        cmd.extend(
            [
                "--available-tools=read_agent,rg,view,web_fetch",
                "--allow-tool=read_agent",
                "--allow-tool=rg",
                "--allow-tool=view",
                "--allow-tool=web_fetch",
            ]
        )
        if args.web_search:
            cmd.append("--allow-all-urls")
        result = engines.run_with_heartbeat(
            cmd,
            Path(tempdir),
            label="copilot",
            stream_output=args.stream_engine_output,
        )
    if result.returncode != 0:
        raise SystemExit(f"copilot engine failed ({result.returncode})\n{result.stderr or result.stdout}")
    return result.stdout
