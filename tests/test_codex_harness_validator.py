import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import validate_codex_harness as validator


def write_valid_harness(root: Path) -> None:
    (root / ".codex" / "agents").mkdir(parents=True)
    (root / ".codex" / "hooks").mkdir(parents=True)
    (root / ".agents" / "skills").mkdir(parents=True)

    shutil.copy(ROOT / ".codex" / "config.toml", root / ".codex" / "config.toml")
    shutil.copy(ROOT / ".codex" / "hooks.json", root / ".codex" / "hooks.json")
    shutil.copy(
        ROOT / ".codex" / "hooks" / "pre_tool_use_guard.py",
        root / ".codex" / "hooks" / "pre_tool_use_guard.py",
    )
    shutil.copy(
        ROOT / ".codex" / "hooks" / "stop_closeout_check.py",
        root / ".codex" / "hooks" / "stop_closeout_check.py",
    )

    for agent_path in (ROOT / ".codex" / "agents").glob("*.toml"):
        shutil.copy(agent_path, root / ".codex" / "agents" / agent_path.name)

    for skill_path in (ROOT / ".agents" / "skills").glob("*/SKILL.md"):
        target = root / ".agents" / "skills" / skill_path.parent.name
        target.mkdir()
        shutil.copy(skill_path, target / "SKILL.md")


class CodexHarnessValidatorTest(unittest.TestCase):
    def test_current_harness_is_valid(self) -> None:
        self.assertEqual([], validator.validate_repo(ROOT))

    def test_rejects_unknown_config_key(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            config_path = root / ".codex" / "config.toml"
            config_path.write_text(
                config_path.read_text(encoding="utf-8").replace(
                    "\n[sandbox_workspace_write]",
                    "\nunknown_key = true\n\n[sandbox_workspace_write]",
                ),
                encoding="utf-8",
            )

            errors = validator.validate_repo(root)

        self.assertIn(".codex/config.toml: unknown root key 'unknown_key'", errors)

    def test_rejects_unknown_hook_key(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            hooks_path = root / ".codex" / "hooks.json"
            payload = json.loads(hooks_path.read_text(encoding="utf-8"))
            payload["hooks"]["PreToolUse"][0]["extra"] = True
            hooks_path.write_text(json.dumps(payload), encoding="utf-8")

            errors = validator.validate_repo(root)

        self.assertIn(".codex/hooks.json: hooks.PreToolUse[0]: unknown key 'extra'", errors)

    def test_rejects_missing_hook_script(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            hooks_path = root / ".codex" / "hooks.json"
            payload = json.loads(hooks_path.read_text(encoding="utf-8"))
            payload["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = (
                'python ".codex/hooks/missing.py"'
            )
            hooks_path.write_text(json.dumps(payload), encoding="utf-8")

            errors = validator.validate_repo(root)

        self.assertIn(
            ".codex/hooks.json: hook command references missing script .codex/hooks/missing.py",
            errors,
        )

    def test_rejects_agent_name_file_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            agent_path = root / ".codex" / "agents" / "docs-scout.toml"
            agent_path.write_text(
                agent_path.read_text(encoding="utf-8").replace(
                    'name = "docs_scout"', 'name = "wrong_name"'
                ),
                encoding="utf-8",
            )

            errors = validator.validate_repo(root)

        self.assertIn(
            ".codex/agents/docs-scout.toml: name must be 'docs_scout'",
            errors,
        )

    def test_rejects_unknown_agent_key(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            agent_path = root / ".codex" / "agents" / "repo-mapper.toml"
            agent_path.write_text(
                agent_path.read_text(encoding="utf-8") + '\nunknown_key = "drift"\n',
                encoding="utf-8",
            )

            errors = validator.validate_repo(root)

        self.assertIn(
            ".codex/agents/repo-mapper.toml: unknown key 'unknown_key'",
            errors,
        )

    def test_rejects_skill_name_file_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            skill_path = root / ".agents" / "skills" / "tdd" / "SKILL.md"
            skill_path.write_text(
                skill_path.read_text(encoding="utf-8").replace(
                    "name: tdd", "name: test_driven"
                ),
                encoding="utf-8",
            )

            errors = validator.validate_repo(root)

        self.assertIn(".agents/skills/tdd/SKILL.md: name must be 'tdd'", errors)

    def test_rejects_skill_without_canonical_rules_reference(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            skill_path = root / ".agents" / "skills" / "tdd" / "SKILL.md"
            skill_path.write_text(
                skill_path.read_text(encoding="utf-8").replace("AGENTS.md", "RULES.md"),
                encoding="utf-8",
            )

            errors = validator.validate_repo(root)

        self.assertIn(
            ".agents/skills/tdd/SKILL.md: must reference AGENTS.md or canonical repo rules",
            errors,
        )

    def test_pre_tool_guard_accepts_command_and_cmd_envelopes(self) -> None:
        for key in ("command", "cmd"):
            with self.subTest(key=key):
                result = subprocess.run(
                    [sys.executable, str(ROOT / ".codex" / "hooks" / "pre_tool_use_guard.py")],
                    input=json.dumps({"tool_input": {key: "rm -rf .git"}}),
                    text=True,
                    capture_output=True,
                    check=False,
                )

                self.assertEqual(0, result.returncode)
                self.assertIn('"permissionDecision": "deny"', result.stdout)


if __name__ == "__main__":
    unittest.main()
