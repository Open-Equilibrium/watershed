#!/usr/bin/env python3
"""PreToolUse(Bash) guard: DENY clearly destructive or protected-path Bash
commands; WARN on risky ones.

Defense-in-depth ONLY. Codex PreToolUse currently intercepts just simple Bash
calls and fails open (the model can bypass by writing a script and running it).
This is a guardrail, NOT a security boundary. M1's modeled policy enforcement
and the post-M1 OS sandbox plan are defined in SECURITY.md. Aligns with its
protected paths (.git, .loop, secrets) and .gitignore. See ADR-0024.
"""
import json
import re
import sys

# Clear-danger patterns -> deny the Bash call.
DENY = [
    (re.compile(r"\brm\s+(?:-\S+\s+)*(?:/|~|\*|\.)\s*$"),
     "Refusing rm targeting root / home / '*' / '.'."),
    (re.compile(r"\brm\b[^|;&]*\.(?:git|loop|clawpatch)\b"),
     "Refusing to delete a protected path (.git / .loop / .clawpatch)."),
    (re.compile(r"(?:^|[\s;&|])>>?\s*\S*\.(?:git|env)\b"),
     "Refusing to redirect/overwrite into a protected path (.git / .env)."),
    (re.compile(r"\b(?:rm|mv|cp|truncate)\b[^|;&]*(?:\.env\b|\.local\b|secret|credential|\.pem\b|\.key\b)"),
     "Refusing to modify a secrets/credentials file."),
    (re.compile(r"\bgit\s+push\b[^|;&]*(?:--force(?!-with-lease)|\s-f\b)"),
     "Refusing git push --force (use --force-with-lease and human review)."),
    (re.compile(r"\b(?:curl|wget)\b[^|]*\|\s*(?:sudo\s+)?(?:ba)?sh\b"),
     "Refusing to pipe network content into a shell (injection/exfiltration risk)."),
]

# Risky-but-sometimes-legitimate -> allow, but surface a warning.
WARN = [
    (re.compile(r"\bgit\s+reset\s+--hard\b"), "git reset --hard can discard work."),
    (re.compile(r"\bgit\s+clean\s+-[a-z]*f"), "git clean -f deletes untracked files."),
    (re.compile(r"\bchmod\s+-R\b"), "recursive chmod is broad; scope it tightly."),
]


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except Exception:
        return 0  # cannot parse -> fail open (guard is advisory only)

    tool_input = data.get("tool_input") or {}
    cmd = tool_input.get("command") or tool_input.get("cmd") or ""

    for rx, reason in DENY:
        if rx.search(cmd):
            json.dump(
                {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": "Watershed guard: " + reason,
                    }
                },
                sys.stdout,
            )
            return 0

    for rx, msg in WARN:
        if rx.search(cmd):
            json.dump({"systemMessage": "Watershed guard: " + msg}, sys.stdout)
            return 0

    return 0  # allow


if __name__ == "__main__":
    sys.exit(main())
