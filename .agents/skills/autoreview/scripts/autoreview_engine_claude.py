from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import autoreview_engines as engines
from autoreview_report import SCHEMA

def run_claude(args: argparse.Namespace, repo: Path, prompt: str) -> str:
    cmd = [
        engines.resolve_command(args.claude_bin, repo),
        "--print",
        "--no-session-persistence",
        "--output-format",
        "stream-json" if args.stream_engine_output else "json",
        "--json-schema",
        json.dumps(SCHEMA),
    ]
    if args.tools:
        cmd.extend(["--allowedTools", claude_allowed_tools(args)])
    else:
        cmd.extend(["--tools", ""])
    if args.stream_engine_output:
        cmd.append("--verbose")
    if args.model:
        cmd.extend(["--model", args.model])
    if args.thinking:
        cmd.extend(["--effort", args.thinking])
    result = engines.run_with_heartbeat(
        cmd,
        repo,
        input_text=prompt,
        label="claude",
        stream_output=args.stream_engine_output,
        stream_display=ClaudeStreamDisplay() if args.stream_engine_output else None,
    )
    if result.returncode != 0:
        raise SystemExit(f"claude engine failed ({result.returncode})\n{result.stderr or result.stdout}")
    return result.stdout

class ClaudeStreamDisplay(engines.JsonStreamDisplay):
    def __init__(self, *, activity_seconds: int = 20) -> None:
        super().__init__("claude", activity_seconds=activity_seconds)
        self.started = False

    def json_event(self, event: dict[str, Any]) -> str | None:
        event_type = event.get("type")
        if event_type == "system" and not self.started:
            self.started = True
            return self.visible("claude turn started\n")
        if event_type == "assistant":
            return self.assistant_message(event)
        if event_type == "result":
            return self.visible(self.flush_hidden() + self.result_summary(event))
        return self.hidden_activity()

    def assistant_message(self, event: dict[str, Any]) -> str | None:
        message = event.get("message")
        if not isinstance(message, dict):
            return self.hidden_activity()
        chunks: list[str] = []
        for item in message.get("content", []):
            if not isinstance(item, dict):
                continue
            if item.get("type") == "text" and isinstance(item.get("text"), str):
                chunks.append(item["text"].rstrip())
        if chunks:
            return self.visible(self.flush_hidden() + "\n".join(chunks) + "\n")
        return self.hidden_activity()

    def result_summary(self, event: dict[str, Any]) -> str:
        usage = event.get("usage")
        fields: list[str] = []
        if isinstance(usage, dict):
            for key in (
                "input_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
                "output_tokens",
            ):
                value = usage.get(key)
                if isinstance(value, int):
                    fields.append(f"{key}={value}")
        cost = event.get("total_cost_usd")
        if isinstance(cost, (int, float)) and not isinstance(cost, bool):
            fields.append(f"cost_usd={cost:.6f}")
        return "claude usage: " + " ".join(fields) + "\n" if fields else "claude turn completed\n"

def claude_allowed_tools(args: argparse.Namespace) -> str:
    tools = [tool.strip() for tool in args.claude_allowed_tools.split(",") if tool.strip()]
    if not args.web_search:
        tools = [tool for tool in tools if tool not in {"WebSearch", "WebFetch"}]
    return ",".join(tools)
