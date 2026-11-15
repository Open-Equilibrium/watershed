#!/usr/bin/env python3
"""A deterministic ACP agent for the T3 contract tests; no model and no account.

The first prompt line that is a script line decides, up to any tag T3 appends to it:
"append <text>" appends <text> to contract.txt in the session's folder through one tool call and answers "Appended."; "wait" runs until T3 cancels
it; anything else answers "Done."."""
import json
import os
import sys
import threading

sessions = {}
cancelled = {}


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def update(sid, body):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": body}})


def say(sid, text):
    update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})


def prompt(mid, sid, script):
    if script.startswith("append "):
        path = os.path.join(sessions[sid], "contract.txt")
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": f"t{mid}", "title": "Append contract.txt",
                     "kind": "edit", "status": "in_progress", "locations": [{"path": path}]})
        with open(path, "a", encoding="utf-8") as f:
            f.write(script[len("append "):] + "\n")
        update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": f"t{mid}", "status": "completed"})
        say(sid, "Appended.")
        reason = "end_turn"
    elif script == "wait":
        cancelled[sid].wait()
        reason = "cancelled"
    else:
        say(sid, "Done.")
        reason = "end_turn"
    send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": reason}})


for raw in sys.stdin:
    if not raw.strip():
        continue
    msg = json.loads(raw)
    method, mid, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": 1, "agentCapabilities": {"loadSession": False}, "authMethods": [],
            "agentInfo": {"name": "scripted", "version": "0.0.1"}}})
    elif method == "session/new":
        sid = f"s{len(sessions) + 1}"
        sessions[sid] = params.get("cwd") or os.getcwd()
        cancelled[sid] = threading.Event()
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": sid}})
    elif method == "session/prompt":
        sid = params["sessionId"]
        cancelled[sid].clear()
        text = "".join(p.get("text", "") for p in params.get("prompt", []) if p.get("type") == "text")
        lines = [line.split("<")[0].strip() for line in text.splitlines()]
        script = next((line for line in lines if line.startswith("append ") or line == "wait"), "")
        threading.Thread(target=prompt, args=(mid, sid, script)).start()
    elif method == "session/cancel":
        cancelled[params["sessionId"]].set()
    elif mid is not None:
        # Optional ACP methods (set_mode, set_model ...): accepted without effect.
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
