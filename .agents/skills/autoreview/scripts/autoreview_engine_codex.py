from __future__ import annotations

import argparse
import json
import tempfile
from pathlib import Path
from typing import Any

import autoreview_engines as engines
from autoreview_report import SCHEMA

def write_json_temp(data: dict[str, Any]) -> Path:
    handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
    with handle:
        json.dump(data, handle)
    return Path(handle.name)

def run_codex(args: argparse.Namespace, repo: Path, prompt: str) -> str:
    if not args.tools:
        raise SystemExit("--no-tools is not supported by the Codex engine; use --engine claude --no-tools for a no-tools run")
    schema_path = write_json_temp(SCHEMA)
    output_path = Path(tempfile.NamedTemporaryFile("w", suffix=".json", delete=False).name)
    try:
        cmd = [engines.resolve_command(args.codex_bin, repo), "--ask-for-approval", "never"]
        if args.web_search:
            cmd.append("--search")
        if args.model:
            cmd.extend(["--model", args.model])
        if args.thinking:
            cmd.extend(["-c", f'model_reasoning_effort="{args.thinking}"'])
        cmd.append("exec")
        if args.stream_engine_output:
            cmd.append("--json")
        cmd.extend(
            [
                "--ephemeral",
                "-C",
                str(repo),
                "-s",
                "read-only",
                "--output-schema",
                str(schema_path),
                "--output-last-message",
                str(output_path),
                "-",
            ]
        )
        result = engines.run_with_heartbeat(
            cmd,
            repo,
            input_text=prompt,
            label="codex",
            stream_output=args.stream_engine_output,
            stream_display=CodexStreamDisplay() if args.stream_engine_output else None,
        )
        output = output_path.read_text(encoding="utf-8")
        if result.returncode != 0:
            raise SystemExit(f"codex engine failed ({result.returncode})\n{result.stderr or result.stdout}")
        return output or result.stdout
    finally:
        schema_path.unlink(missing_ok=True)
        output_path.unlink(missing_ok=True)

class CodexStreamDisplay(engines.JsonStreamDisplay):
    def __init__(self, *, activity_seconds: int = 20) -> None:
        super().__init__("codex", activity_seconds=activity_seconds)

    def json_event(self, event: dict[str, Any]) -> str | None:
        event_type = event.get("type")
        if event_type == "thread.started":
            return self.visible(f"codex thread: {event.get('thread_id', '<unknown>')}\n")
        if event_type == "turn.started":
            return self.visible("codex turn started\n")
        if event_type == "turn.completed":
            usage = event.get("usage")
            message = format_codex_usage(usage) + "\n" if isinstance(usage, dict) else "codex turn completed\n"
            return self.visible(self.flush_hidden() + message)
        item = event.get("item")
        if isinstance(item, dict) and item.get("type") == "agent_message" and isinstance(item.get("text"), str):
            return self.visible(self.flush_hidden() + item["text"].rstrip() + "\n")
        return self.hidden_activity()

def format_codex_usage(usage: dict[str, Any]) -> str:
    fields = [
        "input_tokens",
        "cached_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
    ]
    parts = [f"{field}={usage[field]}" for field in fields if isinstance(usage.get(field), int)]
    return "codex usage: " + " ".join(parts) if parts else "codex usage: unavailable"
