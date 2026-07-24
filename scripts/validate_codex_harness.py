#!/usr/bin/env python3
"""Validate Watershed's local Codex harness files."""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any

CONFIG_ROOT_KEYS = {
    "agents",
    "approval_policy",
    "features",
    "model",
    "model_auto_compact_token_limit",
    "model_reasoning_effort",
    "model_verbosity",
    "personality",
    "sandbox_mode",
    "sandbox_workspace_write",
    "tool_output_token_limit",
    "web_search",
}
CONFIG_TABLE_KEYS = {
    "agents": {"max_depth", "max_threads"},
    "features": {
        "codex_git_commit",
        "fast_mode",
        "goals",
        "hooks",
        "multi_agent",
        "personality",
        "prevent_idle_sleep",
        "terminal_resize_reflow",
        "undo",
    },
    "sandbox_workspace_write": {"network_access"},
}
EXPECTED_CONFIG_VALUES = {
    "approval_policy": "never",
    "sandbox_mode": "workspace-write",
    "web_search": "disabled",
}

REQUIRED_AGENT_FILES = {
    "autoreview_lite.toml": "autoreview_lite",
    "autoreview_pro.toml": "autoreview_pro",
    "clawpatch_lite.toml": "clawpatch_lite",
    "clawpatch_pro.toml": "clawpatch_pro",
    "doc_sync.toml": "doc_sync",
    "docs_scout.toml": "docs_scout",
    "repo_mapper.toml": "repo_mapper",
}
AGENT_KEYS = {
    "description",
    "developer_instructions",
    "model",
    "model_reasoning_effort",
    "name",
    "nickname_candidates",
    "sandbox_mode",
}

REQUIRED_SKILLS = {"autoreview", "clawpatch", "git"}
RETIRED_SKILLS = {"tdd"}
SKILL_FRONT_MATTER_KEYS = {"description", "name"}
HOOK_SCRIPT_PATH_PATTERN = (
    r"\.codex/hooks/(?:[A-Za-z0-9_.-]+/)*[A-Za-z0-9_.-]+\.py"
)
HOOK_COMMAND_RE = re.compile(
    rf'^node (?:"scripts/run-python\.mjs"|scripts/run-python\.mjs) '
    rf'(?:"(?P<quoted_script>{HOOK_SCRIPT_PATH_PATTERN})"|'
    rf'(?P<script>{HOOK_SCRIPT_PATH_PATTERN}))$'
)
HOOK_SCRIPT_BY_EVENT = {
    "PreToolUse": ".codex/hooks/pre_tool_use_guard.py",
    "Stop": ".codex/hooks/stop_closeout_check.py",
}


def validate_repo(root: Path) -> list[str]:
    root = root.resolve()
    errors: list[str] = []
    errors.extend(validate_config(root))
    errors.extend(validate_hooks(root))
    errors.extend(validate_agents(root))
    errors.extend(validate_skills(root))
    return errors


def validate_config(root: Path) -> list[str]:
    path = root / ".codex" / "config.toml"
    rel = ".codex/config.toml"
    errors: list[str] = []
    config = read_toml(path, rel, errors)
    if config is None:
        return errors

    errors.extend(unknown_keys(rel, config, CONFIG_ROOT_KEYS, "root key"))
    for table, allowed_keys in CONFIG_TABLE_KEYS.items():
        value = config.get(table)
        if not isinstance(value, dict):
            errors.append(f"{rel}: [{table}] must be a table")
            continue
        errors.extend(unknown_keys(f"{rel}[{table}]", value, allowed_keys, "key"))

    for key, expected in EXPECTED_CONFIG_VALUES.items():
        if config.get(key) != expected:
            errors.append(f"{rel}: {key} must be {expected!r}")
    sandbox_workspace_write = config.get("sandbox_workspace_write")
    network_access = (
        sandbox_workspace_write.get("network_access")
        if isinstance(sandbox_workspace_write, dict)
        else None
    )
    if network_access is not True:
        errors.append(f"{rel}: sandbox_workspace_write.network_access must be true")
    features = config.get("features")
    if not isinstance(features, dict):
        features = {}
    if features.get("hooks") is not True:
        errors.append(f"{rel}: features.hooks must be true")
    if features.get("multi_agent") is not True:
        errors.append(f"{rel}: features.multi_agent must be true")
    if features.get("codex_git_commit") is not False:
        errors.append(f"{rel}: features.codex_git_commit must be false")
    return errors


def validate_hooks(root: Path) -> list[str]:
    path = root / ".codex" / "hooks.json"
    rel = ".codex/hooks.json"
    errors: list[str] = []
    text = read_utf8(path, rel, errors)
    if text is None:
        return errors
    try:
        data = json.loads(text)
    except json.JSONDecodeError as err:
        return [f"{rel}: invalid JSON: {err.msg}"]

    if not isinstance(data, dict):
        return [f"{rel}: root must be an object"]

    errors.extend(unknown_keys(rel, data, {"hooks"}, "root key"))
    hooks = data.get("hooks")
    if not isinstance(hooks, dict):
        return errors + [f"{rel}: hooks must be an object"]

    expected_events = {"PreToolUse", "Stop"}
    errors.extend(unknown_keys(rel, hooks, expected_events, "hook event"))
    for event in expected_events:
        entries = hooks.get(event)
        if not isinstance(entries, list) or not entries:
            errors.append(f"{rel}: hooks.{event} must be a non-empty list")
            continue
        for entry_index, entry in enumerate(entries):
            errors.extend(validate_hook_entry(root, rel, event, entry_index, entry))
    return errors


def validate_hook_entry(
    root: Path,
    rel: str,
    event: str,
    entry_index: int,
    entry: Any,
) -> list[str]:
    prefix = f"{rel}: hooks.{event}[{entry_index}]"
    if not isinstance(entry, dict):
        return [f"{prefix} must be an object"]

    allowed_entry_keys = {"hooks", "matcher"} if event == "PreToolUse" else {"hooks"}
    errors = unknown_keys(prefix, entry, allowed_entry_keys, "key")
    if event == "PreToolUse" and entry.get("matcher") != "Bash":
        errors.append(f"{rel}: PreToolUse matcher must be 'Bash'")
    inner_hooks = entry.get("hooks")
    if not isinstance(inner_hooks, list) or not inner_hooks:
        return errors + [f"{prefix}.hooks must be a non-empty list"]
    for hook_index, hook in enumerate(inner_hooks):
        errors.extend(validate_hook_command(root, rel, event, entry_index, hook_index, hook))
    return errors


def validate_hook_command(
    root: Path,
    rel: str,
    event: str,
    entry_index: int,
    hook_index: int,
    hook: Any,
) -> list[str]:
    prefix = f"{rel}: hooks.{event}[{entry_index}].hooks[{hook_index}]"
    if not isinstance(hook, dict):
        return [f"{prefix} must be an object"]

    errors = unknown_keys(prefix, hook, {"command", "statusMessage", "timeout", "type"}, "key")
    if hook.get("type") != "command":
        errors.append(f"{prefix}.type must be 'command'")
    command = hook.get("command")
    if not isinstance(command, str) or not command:
        return errors + [f"{prefix}.command must be a non-empty string"]
    if re.search(r"(?:^|\s)(?:bash|sh)\s+-c(?:\s|$)", command):
        errors.append(f"{rel}: hook command must not require a POSIX shell")
    timeout = hook.get("timeout")
    if type(timeout) is not int or timeout <= 0:
        errors.append(f"{prefix}.timeout must be a positive integer")

    match = HOOK_COMMAND_RE.fullmatch(command)
    if match is None:
        errors.append(f"{rel}: hook command must use the approved Node launcher form")
        return errors

    script = match.group("quoted_script") or match.group("script")
    expected_script = HOOK_SCRIPT_BY_EVENT[event]
    if script != expected_script:
        errors.append(f"{prefix}.command must run {expected_script}")
    resolved_root = root.resolve()
    script_path = (root / script).resolve()
    hooks_dir = (root / ".codex" / "hooks").resolve()
    if not hooks_dir.is_relative_to(resolved_root) or not script_path.is_relative_to(
        resolved_root
    ):
        errors.append(f"{rel}: hook command script must stay within repository")
    elif not script_path.is_relative_to(hooks_dir):
        errors.append(f"{rel}: hook command script must be below .codex/hooks")
    elif not script_path.is_file():
        errors.append(f"{rel}: hook command references missing script {script}")
    return errors


def validate_agents(root: Path) -> list[str]:
    agent_dir = root / ".codex" / "agents"
    errors: list[str] = []
    if not agent_dir.is_dir():
        return [".codex/agents: missing directory"]

    found_files = {path.name for path in agent_dir.glob("*.toml")}
    for filename in sorted(set(REQUIRED_AGENT_FILES) - found_files):
        errors.append(f".codex/agents/{filename}: missing required agent")

    for path in sorted(agent_dir.glob("*.toml")):
        rel = f".codex/agents/{path.name}"
        agent = read_toml(path, rel, errors)
        if agent is None:
            continue
        errors.extend(unknown_keys(rel, agent, AGENT_KEYS, "key"))

        expected_name = REQUIRED_AGENT_FILES.get(path.name, path.stem.replace("-", "_"))
        if agent.get("name") != expected_name:
            errors.append(f"{rel}: name must be {expected_name!r}")
        nicknames = agent.get("nickname_candidates")
        if nicknames is not None and (
            not isinstance(nicknames, list)
            or not nicknames
            or not all(isinstance(nickname, str) and nickname for nickname in nicknames)
        ):
            errors.append(f"{rel}: nickname_candidates must be a non-empty string list")
        instructions = agent.get("developer_instructions")
        if not isinstance(instructions, str) or "AGENTS.md" not in instructions:
            errors.append(f"{rel}: developer_instructions must reference AGENTS.md")
        elif agent.get("name") == "docs_scout" and "docs/adr/ADR-LOG.md" not in instructions:
            errors.append(f"{rel}: docs_scout must reference docs/adr/ADR-LOG.md")
        elif agent.get("name") == "doc_sync" and "docs/decisions/open-decisions.html" not in instructions:
            errors.append(f"{rel}: doc_sync must reference docs/decisions/open-decisions.html")
    return errors


def validate_skills(root: Path) -> list[str]:
    skill_dir = root / ".agents" / "skills"
    errors: list[str] = []
    if not skill_dir.is_dir():
        return [".agents/skills: missing directory"]

    found_skill_dirs = {path.name for path in skill_dir.iterdir() if path.is_dir()}
    found_skills = {path.parent.name for path in skill_dir.glob("*/SKILL.md")}
    for name in sorted(REQUIRED_SKILLS - found_skills):
        errors.append(f".agents/skills/{name}/SKILL.md: missing required skill")
    for name in sorted(RETIRED_SKILLS & found_skill_dirs):
        errors.append(f".agents/skills/{name}/SKILL.md: retired skill must be absent")

    for path in sorted(skill_dir.glob("*/SKILL.md")):
        rel = f".agents/skills/{path.parent.name}/SKILL.md"
        text = read_utf8(path, rel, errors)
        if text is None:
            continue
        metadata = parse_skill_front_matter(text)
        if metadata is None:
            errors.append(f"{rel}: missing or invalid front matter")
            continue
        errors.extend(unknown_keys(rel, metadata, SKILL_FRONT_MATTER_KEYS, "front matter key"))
        expected_name = path.parent.name
        if metadata.get("name") != expected_name:
            errors.append(f"{rel}: name must be {expected_name!r}")
        if not metadata.get("description"):
            errors.append(f"{rel}: description is required")
        if not references_canonical_rules(text):
            errors.append(f"{rel}: must reference AGENTS.md or canonical repo rules")
    return errors


def read_utf8(path: Path, rel: str, errors: list[str]) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        errors.append(f"{rel}: missing file")
    except UnicodeError:
        errors.append(f"{rel}: file must be valid UTF-8")
    except OSError:
        errors.append(f"{rel}: file could not be read")
    return None


def read_toml(path: Path, rel: str, errors: list[str]) -> dict[str, Any] | None:
    try:
        with path.open("rb") as handle:
            data = tomllib.load(handle)
    except FileNotFoundError:
        errors.append(f"{rel}: missing file")
        return None
    except UnicodeError:
        errors.append(f"{rel}: file must be valid UTF-8")
        return None
    except tomllib.TOMLDecodeError as err:
        errors.append(f"{rel}: invalid TOML: {err}")
        return None
    except OSError:
        errors.append(f"{rel}: file could not be read")
        return None
    return data


def parse_skill_front_matter(text: str) -> dict[str, str] | None:
    if not text.startswith("---\n"):
        return None
    lines = text.splitlines()
    metadata: dict[str, str] = {}
    for line in lines[1:]:
        if line == "---":
            return metadata
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        key, sep, value = line.partition(":")
        if not sep or not key.strip():
            return None
        metadata[key.strip()] = value.strip().strip('"')
    return None


def references_canonical_rules(text: str) -> bool:
    return "AGENTS.md" in text or all(
        token in text for token in ("TESTING.md", "PERFORMANCE.md", "git skill")
    )


def unknown_keys(
    location: str,
    data: dict[str, Any],
    allowed: set[str],
    label: str,
) -> list[str]:
    return [f"{location}: unknown {label} {key!r}" for key in sorted(set(data) - allowed)]


def main() -> int:
    errors = validate_repo(Path.cwd())
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("Codex harness validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
