#!/usr/bin/env python3
"""A deterministic ACP agent for T3 contract tests: each prompt appends one line to
contract.txt in the session's folder and answers with a fixed text. No model, no account."""
import json
import os
import sys

sessions = {}


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def log(text):
    sys.stderr.write(f"scripted-acp: {text}\n")
    sys.stderr.flush()


for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    method, mid, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
    log(f"<- {method or 'response'}")
    if method is None:
        continue
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": 1,
            "agentCapabilities": {"loadSession": False},
            "authMethods": [],
            "agentInfo": {"name": "scripted", "version": "0.0.1"},
        }})
    elif method == "session/new":
        sid = f"s{len(sessions) + 1}"
        sessions[sid] = params.get("cwd")
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": sid}})
    elif method == "session/prompt":
        sid = params["sessionId"]
        text = "".join(p.get("text", "") for p in params.get("prompt", []) if p.get("type") == "text")
        path = os.path.join(sessions.get(sid) or os.getcwd(), "contract.txt")
        with open(path, "a", encoding="utf-8") as f:
            f.write(text.strip().splitlines()[-1][:80] + "\n")
        send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "Appended one line to contract.txt."},
        }}})
        send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
    elif mid is not None:
        # Optional ACP methods (set_mode, set_model, cancel ...): accept without effect.
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
