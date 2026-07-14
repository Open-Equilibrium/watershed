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


def validate_text_replacement(relative_path: str, old: str, new: str) -> list[str]:
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        write_valid_harness(root)
        path = root / relative_path
        text = path.read_text(encoding="utf-8")
        if old not in text:
            raise AssertionError(f"missing {old!r} in {relative_path}")
        path.write_text(text.replace(old, new), encoding="utf-8")
        return validator.validate_repo(root)


def run_node_module_test(source: str) -> subprocess.CompletedProcess[str]:
    node = shutil.which("node")
    if node is None:
        raise AssertionError("node executable is required")
    return subprocess.run(
        [node, "--input-type=module", "-e", source],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=20,
    )


class CodexHarnessValidatorTest(unittest.TestCase):
    def test_current_harness_is_valid(self) -> None:
        self.assertEqual([], validator.validate_repo(ROOT))

    def test_project_python_launcher_runs_python(self) -> None:
        result = subprocess.run(
            [
                "node",
                str(ROOT / "scripts" / "run-python.mjs"),
                "-c",
                "import sys; sys.stdout.write('ok')",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=20,
        )

        self.assertEqual("ok", result.stdout)
        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)

    def test_windows_python_launcher_failure_falls_back_to_python(self) -> None:
        result = run_node_module_test(
            f"""
            import assert from "node:assert/strict";
            import {{ runPython }} from {json.dumps((ROOT / "scripts" / "run-python.mjs").as_uri())};

            const calls = [];
            const stderr = {{ text: "", write(chunk) {{ this.text += chunk; }} }};
            const status = runPython(["script.py"], {{
              platform: "win32",
              stderr,
              spawnSync(executable, args) {{
                calls.push({{ executable, args }});
                if (executable === "py") return {{ status: 103 }};
                if (executable === "python3") {{
                  return {{ error: Object.assign(new Error("missing"), {{ code: "ENOENT" }}) }};
                }}
                if (executable === "python") return {{ status: 0 }};
                throw new Error(`unexpected executable ${{executable}}`);
              }},
            }});

            assert.equal(status, 0);
            assert.equal(stderr.text, "");
            assert.deepEqual(calls.map((call) => call.executable), ["py", "python3", "python"]);
            assert.deepEqual(calls[0].args.slice(0, 2), ["-3", "-c"]);
            """
        )

        self.assertEqual("", result.stdout)
        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)

    def test_windows_python_launcher_preserves_script_failure(self) -> None:
        result = run_node_module_test(
            f"""
            import assert from "node:assert/strict";
            import {{ runPython }} from {json.dumps((ROOT / "scripts" / "run-python.mjs").as_uri())};

            const calls = [];
            const stderr = {{ text: "", write(chunk) {{ this.text += chunk; }} }};
            const status = runPython(["script.py"], {{
              platform: "win32",
              stderr,
              spawnSync(executable, args) {{
                calls.push({{ executable, args }});
                if (executable !== "py") throw new Error(`unexpected fallback ${{executable}}`);
                return calls.length === 1 ? {{ status: 0 }} : {{ status: 7 }};
              }},
            }});

            assert.equal(status, 7);
            assert.equal(stderr.text, "");
            assert.deepEqual(calls.map((call) => call.executable), ["py", "py"]);
            assert.deepEqual(calls[0].args.slice(0, 2), ["-3", "-c"]);
            assert.deepEqual(calls[1].args, ["-3", "script.py"]);
            """
        )

        self.assertEqual("", result.stdout)
        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)

    def test_rejects_harness_text_drift(self) -> None:
        cases = [
            (
                "unknown config key",
                ".codex/config.toml",
                "\n[sandbox_workspace_write]",
                "\nunknown_key = true\n\n[sandbox_workspace_write]",
                ".codex/config.toml: unknown root key 'unknown_key'",
            ),
            (
                "disabled network",
                ".codex/config.toml",
                "network_access = true",
                "network_access = false",
                ".codex/config.toml: sandbox_workspace_write.network_access must be true",
            ),
            (
                "non-boolean network",
                ".codex/config.toml",
                "network_access = true",
                'network_access = "true"',
                ".codex/config.toml: sandbox_workspace_write.network_access must be true",
            ),
            (
                "agent name",
                ".codex/agents/docs-scout.toml",
                'name = "docs_scout"',
                'name = "wrong_name"',
                ".codex/agents/docs-scout.toml: name must be 'docs_scout'",
            ),
            (
                "unknown agent key",
                ".codex/agents/repo-mapper.toml",
                'sandbox_mode = "read-only"',
                'sandbox_mode = "read-only"\nunknown_key = "drift"',
                ".codex/agents/repo-mapper.toml: unknown key 'unknown_key'",
            ),
            (
                "skill name",
                ".agents/skills/tdd/SKILL.md",
                "name: tdd",
                "name: test_driven",
                ".agents/skills/tdd/SKILL.md: name must be 'tdd'",
            ),
            (
                "skill rules reference",
                ".agents/skills/tdd/SKILL.md",
                "AGENTS.md",
                "RULES.md",
                ".agents/skills/tdd/SKILL.md: must reference AGENTS.md or canonical repo rules",
            ),
            (
                "validator gate reference",
                ".codex/agents/pr-validator.toml",
                "TESTING.md",
                "GATES.md",
                ".codex/agents/pr-validator.toml: pr_validator must reference TESTING.md",
            ),
        ]

        for name, path, old, new, expected in cases:
            with self.subTest(name=name):
                self.assertIn(expected, validate_text_replacement(path, old, new))

    def test_rejects_hook_drift(self) -> None:
        cases = [
            (
                "unknown key",
                "entry",
                "extra",
                True,
                ".codex/hooks.json: hooks.PreToolUse[0]: unknown key 'extra'",
            ),
            (
                "missing script",
                "hook",
                "command",
                'python ".codex/hooks/missing.py"',
                ".codex/hooks.json: hook command references missing script .codex/hooks/missing.py",
            ),
            (
                "POSIX shell",
                "hook",
                "command",
                'sh -c \'exec python "$1"\' sh ".codex/hooks/pre_tool_use_guard.py"',
                ".codex/hooks.json: hook command must not require a POSIX shell",
            ),
        ]

        for name, target, key, value, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                write_valid_harness(root)
                hooks_path = root / ".codex" / "hooks.json"
                payload = json.loads(hooks_path.read_text(encoding="utf-8"))
                entry = payload["hooks"]["PreToolUse"][0]
                (entry if target == "entry" else entry["hooks"][0])[key] = value
                hooks_path.write_text(json.dumps(payload), encoding="utf-8")

                self.assertIn(expected, validator.validate_repo(root))

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
