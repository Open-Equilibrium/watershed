#!/usr/bin/env python3
import json
import os
import re
import sys
import threading

sessions = {}
cancelled = {}
replies = {}
instructions = {}


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def update(sid, body):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": body}})


def say(sid, text):
    update(sid, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})


def prompt(mid, sid, script):
    if script in ("instruct", "reinstruct"):
        path = instructions[sid]
        twice = " twice" if script == "reinstruct" else ""
        with open(path, "w", encoding="utf-8") as f:
            f.write(f"# news-bot\n\nRead the feeds{twice}.\n")
        say(sid, "Saved.")
        reason = "end_turn"
    elif script == "rewrite-wait":
        with open(instructions[sid], "w", encoding="utf-8") as f:
            f.write("# news-bot\n\nRead the feeds three times.\n")
        cancelled[sid].wait()
        reason = "cancelled"
    elif script.startswith("append "):
        path = os.path.join(sessions[sid], "contract.txt")
        update(sid, {"sessionUpdate": "tool_call", "toolCallId": f"t{mid}", "title": "Append contract.txt",
                     "kind": "edit", "status": "in_progress", "locations": [{"path": path}]})
        with open(path, "a", encoding="utf-8") as f:
            f.write(script[len("append "):] + "\n")
        update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": f"t{mid}", "status": "completed"})
        say(sid, "Appended.")
        reason = "end_turn"
    elif script == "ask":
        asked = threading.Event()
        replies[f"p{mid}"] = [asked, None]
        send({"jsonrpc": "2.0", "id": f"p{mid}", "method": "session/request_permission", "params": {
            "sessionId": sid,
            "toolCall": {"toolCallId": f"t{mid}", "title": "Edit contract.txt", "kind": "edit"},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}})
        asked.wait()
        outcome = replies.pop(f"p{mid}")[1].get("outcome", {})
        say(sid, "Approved." if outcome.get("optionId") == "allow" else "Declined.")
        reason = "end_turn"
    elif script.startswith("stream "):
        for _ in range(int(script[len("stream "):])):
            say(sid, "sixteen bytes ok")
        reason = "end_turn"
    elif script == "plan":
        for status in ("in_progress", "completed"):
            update(sid, {"sessionUpdate": "plan", "entries": [
                {"content": "Explore", "priority": "medium", "status": status},
                {"content": "Explore › scraper-1", "priority": "medium", "status": status}]})
        say(sid, "Planned.")
        reason = "end_turn"
    elif script == "fill":
        update(sid, {"sessionUpdate": "usage_update", "used": 900, "size": 1000})
        say(sid, "Full.")
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
    if method is None and mid in replies:
        replies[mid][1] = msg.get("result") or {}
        replies[mid][0].set()
    elif method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": 1, "agentCapabilities": {"loadSession": False}, "authMethods": [],
            "agentInfo": {"name": "scripted", "version": "0.0.1"}}})
    elif method == "session/new":
        sid = f"s{os.getpid()}-{len(sessions) + 1}"
        sessions[sid] = params.get("cwd") or os.getcwd()
        cancelled[sid] = threading.Event()
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": sid}})
    elif method == "session/prompt":
        sid = params["sessionId"]
        cancelled[sid].clear()
        text = "".join(p.get("text", "") for p in params.get("prompt", []) if p.get("type") == "text")
        named = re.search(r"(/\S+/instructions\.md)", text)
        if named:
            instructions[sid] = named.group(1)
        lines = [line.split("<")[0].strip() for line in text.splitlines()]
        handoff = "Imported conversation history from the previous host" in text
        script = "" if handoff else next((line for line in reversed(lines) if line.startswith(("append ", "stream ")) or line in ("ask", "wait", "plan", "fill", "instruct", "reinstruct", "rewrite-wait")), "")
        threading.Thread(target=prompt, args=(mid, sid, script)).start()
    elif method == "session/cancel":
        cancelled[params["sessionId"]].set()
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
