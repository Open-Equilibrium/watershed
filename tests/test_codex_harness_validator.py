import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import validate_codex_harness as validator

sys.path.insert(0, str(ROOT / ".codex" / "hooks"))
import stop_closeout_check as stop_hook


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

    def test_reports_malformed_config_shapes_without_crashing(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            (root / ".codex" / "config.toml").write_text(
                (root / ".codex" / "config.toml")
                .read_text(encoding="utf-8")
                .replace("[features]", 'features = "invalid"'),
                encoding="utf-8",
            )
            (root / ".codex" / "hooks.json").write_text("[]", encoding="utf-8")

            errors = validator.validate_repo(root)

        self.assertIn(".codex/config.toml: [features] must be a table", errors)
        self.assertIn(".codex/hooks.json: root must be an object", errors)

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

    def test_python_launcher_flushes_usage_error_before_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            error_path = root / "stderr.txt"
            preload_path = root / "async-stderr.cjs"
            preload_path.write_text(
                """
const { writeFileSync } = require("node:fs");
process.stderr.write = (message) => {
  setTimeout(() => writeFileSync(process.env.ERROR_FILE, message), 0);
  return false;
};
""",
                encoding="utf-8",
            )
            env = os.environ | {"ERROR_FILE": str(error_path)}
            result = subprocess.run(
                [
                    "node",
                    "--require",
                    str(preload_path),
                    str(ROOT / "scripts" / "run-python.mjs"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=20,
                env=env,
            )

            self.assertEqual(2, result.returncode)
            self.assertEqual(
                "usage: node scripts/run-python.mjs <python-args...>\n",
                error_path.read_text(encoding="utf-8"),
            )

    def test_non_windows_python_launcher_fallback_and_failures(self) -> None:
        result = run_node_module_test(
            f"""
            import assert from "node:assert/strict";
            import {{ runPython }} from {json.dumps((ROOT / "scripts" / "run-python.mjs").as_uri())};

            const missing = () => ({{
              error: Object.assign(new Error("missing"), {{ code: "ENOENT" }}),
            }});

            const fallbackCalls = [];
            const fallbackStatus = runPython(["script.py"], {{
              platform: "linux",
              spawnSync(executable) {{
                fallbackCalls.push(executable);
                return executable === "python3" ? missing() : {{ status: 0 }};
              }},
            }});
            assert.equal(fallbackStatus, 0);
            assert.deepEqual(fallbackCalls, ["python3", "python"]);

            const failureCalls = [];
            const failureStatus = runPython(["script.py"], {{
              platform: "darwin",
              spawnSync(executable) {{
                failureCalls.push(executable);
                return {{ status: 7 }};
              }},
            }});
            assert.equal(failureStatus, 7);
            assert.deepEqual(failureCalls, ["python3"]);

            const exhaustionCalls = [];
            const stderr = {{ text: "", write(chunk) {{ this.text += chunk; }} }};
            const exhaustionStatus = runPython(["script.py"], {{
              platform: "linux",
              stderr,
              spawnSync(executable) {{
                exhaustionCalls.push(executable);
                return missing();
              }},
            }});
            assert.equal(exhaustionStatus, 127);
            assert.deepEqual(exhaustionCalls, ["python3", "python"]);
            assert.equal(stderr.text, "missing Python interpreter: tried python3, python\\n");
            """
        )

        self.assertEqual("", result.stdout)
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

    def test_windows_python_launcher_stops_on_probe_error_or_signal(self) -> None:
        result = run_node_module_test(
            f"""
            import assert from "node:assert/strict";
            import {{ runPython }} from {json.dumps((ROOT / "scripts" / "run-python.mjs").as_uri())};

            for (const probe of [
              {{ error: Object.assign(new Error("access denied"), {{ code: "EACCES" }}) }},
              {{ signal: "SIGTERM" }},
            ]) {{
              const calls = [];
              const stderr = {{ text: "", write(chunk) {{ this.text += chunk; }} }};
              const status = runPython(["script.py"], {{
                platform: "win32",
                stderr,
                spawnSync(executable) {{
                  calls.push(executable);
                  return probe;
                }},
              }});

              assert.equal(status, 1);
              assert.deepEqual(calls, ["py"]);
              assert.notEqual(stderr.text, "");
            }}
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
                ".codex/agents/docs_scout.toml",
                'name = "docs_scout"',
                'name = "wrong_name"',
                ".codex/agents/docs_scout.toml: name must be 'docs_scout'",
            ),
            (
                "unknown agent key",
                ".codex/agents/repo_mapper.toml",
                'sandbox_mode = "read-only"',
                'sandbox_mode = "read-only"\nunknown_key = "drift"',
                ".codex/agents/repo_mapper.toml: unknown key 'unknown_key'",
            ),
            (
                "skill name",
                ".agents/skills/git/SKILL.md",
                "name: git",
                "name: test_driven",
                ".agents/skills/git/SKILL.md: name must be 'git'",
            ),
            (
                "skill rules reference",
                ".agents/skills/git/SKILL.md",
                "AGENTS.md",
                "RULES.md",
                ".agents/skills/git/SKILL.md: must reference AGENTS.md or canonical repo rules",
            ),
            (
                "agent rules reference",
                ".codex/agents/docs_scout.toml",
                "AGENTS.md",
                "RULES.md",
                ".codex/agents/docs_scout.toml: developer_instructions must reference AGENTS.md",
            ),
        ]

        for name, path, old, new, expected in cases:
            with self.subTest(name=name):
                self.assertIn(expected, validate_text_replacement(path, old, new))

    def test_reports_invalid_agent_instructions_without_crashing(self) -> None:
        for agent in ["docs_scout", "doc_sync"]:
            for replacement in ["", "developer_instructions = 7\n"]:
                with self.subTest(agent=agent, replacement=replacement):
                    with tempfile.TemporaryDirectory() as temp:
                        root = Path(temp)
                        write_valid_harness(root)
                        path = root / ".codex" / "agents" / f"{agent}.toml"
                        text = path.read_text(encoding="utf-8")
                        start = text.index('developer_instructions = """')
                        end = text.index('"""', start + 28) + 3
                        path.write_text(
                            text[:start] + replacement + text[end:], encoding="utf-8"
                        )

                        self.assertIn(
                            f".codex/agents/{agent}.toml: developer_instructions must reference AGENTS.md",
                            validator.validate_repo(root),
                        )

    def test_rejects_missing_required_tiered_agent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            (root / ".codex" / "agents" / "autoreview_lite.toml").unlink()

            self.assertIn(
                ".codex/agents/autoreview_lite.toml: missing required agent",
                validator.validate_repo(root),
            )

    def test_agent_nicknames_are_optional(self) -> None:
        errors = validate_text_replacement(
            ".codex/agents/docs_scout.toml",
            'nickname_candidates = ["Where Is The Spec", "Ctrl F Forever", "Decision Archaeologist"]\n',
            "",
        )

        self.assertEqual([], errors)

    def test_rejects_retired_tdd_skill(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            target = root / ".agents" / "skills" / "tdd"
            target.mkdir()
            (target / "SKILL.md").write_text(
                '---\nname: tdd\ndescription: "retired"\n---\n\nSee AGENTS.md.\n',
                encoding="utf-8",
            )

            self.assertIn(
                ".agents/skills/tdd/SKILL.md: retired skill must be absent",
                validator.validate_repo(root),
            )

    def test_rejects_required_skill_directory_without_skill_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            (root / ".agents" / "skills" / "clawpatch" / "SKILL.md").unlink()

            self.assertIn(
                ".agents/skills/clawpatch/SKILL.md: missing required skill",
                validator.validate_repo(root),
            )

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
                'node "scripts/run-python.mjs" ".codex/hooks/missing.py"',
                ".codex/hooks.json: hook command references missing script .codex/hooks/missing.py",
            ),
            (
                "boolean timeout",
                "hook",
                "timeout",
                True,
                ".codex/hooks.json: hooks.PreToolUse[0].hooks[0].timeout must be a positive integer",
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

    def test_rejects_hook_commands_outside_approved_scripts(self) -> None:
        cases = [
            (
                "chained command",
                'node "scripts/run-python.mjs" ".codex/hooks/pre_tool_use_guard.py" && node "scripts/run-python.mjs" ".codex/hooks/stop_closeout_check.py"',
                ".codex/hooks.json: hook command must use the approved Node launcher form",
                False,
            ),
            (
                "script traversal",
                'node "scripts/run-python.mjs" ".codex/hooks/../../outside.py"',
                ".codex/hooks.json: hook command script must be below .codex/hooks",
                True,
            ),
        ]

        for name, command, expected, create_outside_script in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                write_valid_harness(root)
                if create_outside_script:
                    (root / "outside.py").write_text("", encoding="utf-8")
                hooks_path = root / ".codex" / "hooks.json"
                payload = json.loads(hooks_path.read_text(encoding="utf-8"))
                payload["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = command
                hooks_path.write_text(json.dumps(payload), encoding="utf-8")

                self.assertIn(expected, validator.validate_repo(root))

    def test_rejects_symlinked_hooks_directory_outside_repository(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "repo"
            external_hooks = Path(temp) / "external-hooks"
            write_valid_harness(root)
            shutil.copytree(root / ".codex" / "hooks", external_hooks)
            shutil.rmtree(root / ".codex" / "hooks")
            try:
                (root / ".codex" / "hooks").symlink_to(
                    external_hooks,
                    target_is_directory=True,
                )
            except OSError as error:
                if os.name != "nt":
                    self.skipTest(f"directory symlinks unavailable: {error}")
                result = subprocess.run(
                    [
                        "cmd",
                        "/c",
                        "mklink",
                        "/J",
                        str(root / ".codex" / "hooks"),
                        str(external_hooks),
                    ],
                    capture_output=True,
                    text=True,
                    timeout=20,
                )
                if result.returncode != 0:
                    self.skipTest(f"directory links unavailable: {result.stderr}")

            self.assertIn(
                ".codex/hooks.json: hook command script must stay within repository",
                validator.validate_repo(root),
            )

    def test_rejects_hooks_registered_for_the_wrong_events(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_valid_harness(root)
            hooks_path = root / ".codex" / "hooks.json"
            payload = json.loads(hooks_path.read_text(encoding="utf-8"))
            pre_tool_hook = payload["hooks"]["PreToolUse"][0]["hooks"][0]
            stop_hook = payload["hooks"]["Stop"][0]["hooks"][0]
            pre_tool_hook["command"], stop_hook["command"] = (
                stop_hook["command"],
                pre_tool_hook["command"],
            )
            hooks_path.write_text(json.dumps(payload), encoding="utf-8")

            errors = validator.validate_repo(root)

            self.assertIn(
                ".codex/hooks.json: hooks.PreToolUse[0].hooks[0].command must run "
                ".codex/hooks/pre_tool_use_guard.py",
                errors,
            )
            self.assertIn(
                ".codex/hooks.json: hooks.Stop[0].hooks[0].command must run "
                ".codex/hooks/stop_closeout_check.py",
                errors,
            )

    def test_rejects_shell_metacharacters_in_hook_script_paths(self) -> None:
        commands = [
            'node "scripts/run-python.mjs" ".codex/hooks/$(touch pwn).py"',
            'node "scripts/run-python.mjs" ".codex/hooks/`touch pwn`.py"',
            'node "scripts/run-python.mjs" .codex/hooks/unsafe;name.py',
            'node "scripts/run-python.mjs" .codex/hooks/unsafe&name.py',
            'node "scripts/run-python.mjs" .codex/hooks/unsafe|name.py',
            'node "scripts/run-python.mjs" .codex/hooks/unsafe>name.py',
            r'node "scripts/run-python.mjs" .codex\hooks\pre_tool_use_guard.py',
            "node\nscripts/run-python.mjs .codex/hooks/pre_tool_use_guard.py",
        ]

        for command in commands:
            with self.subTest(command=command), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                write_valid_harness(root)
                hooks_path = root / ".codex" / "hooks.json"
                payload = json.loads(hooks_path.read_text(encoding="utf-8"))
                payload["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = command
                hooks_path.write_text(json.dumps(payload), encoding="utf-8")

                self.assertIn(
                    ".codex/hooks.json: hook command must use the approved Node launcher form",
                    validator.validate_repo(root),
                )

    def test_pre_tool_guard_accepts_command_and_cmd_envelopes(self) -> None:
        for key in ("command", "cmd"):
            for protected_path in (".git", ".flow", ".clawpatch"):
                with self.subTest(key=key, protected_path=protected_path):
                    result = subprocess.run(
                        [
                            sys.executable,
                            str(ROOT / ".codex" / "hooks" / "pre_tool_use_guard.py"),
                        ],
                        input=json.dumps(
                            {"tool_input": {key: f"rm -rf {protected_path}"}}
                        ),
                        text=True,
                        capture_output=True,
                        check=False,
                    )

                    self.assertEqual(0, result.returncode)
                    self.assertEqual(
                        {
                            "hookSpecificOutput": {
                                "hookEventName": "PreToolUse",
                                "permissionDecision": "deny",
                                "permissionDecisionReason": (
                                    "Watershed guard: Refusing to delete a protected path "
                                    "(.git / .flow / .clawpatch)."
                                ),
                            }
                        },
                        json.loads(result.stdout),
                    )
                    self.assertEqual("", result.stderr)

    def test_pre_tool_guard_fails_open_for_unexpected_json_shapes(self) -> None:
        cases = [
            [],
            {"tool_input": ["unexpected"]},
            {"tool_input": {"command": ["unexpected"]}},
        ]
        for payload in cases:
            with self.subTest(payload=payload):
                result = subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / ".codex" / "hooks" / "pre_tool_use_guard.py"),
                    ],
                    input=json.dumps(payload),
                    text=True,
                    capture_output=True,
                    check=False,
                )

                self.assertEqual(0, result.returncode)
                self.assertEqual("", result.stdout)
                self.assertEqual("", result.stderr)

    def test_stop_hook_reports_only_ten_conflict_markers(self) -> None:
        conflicts = [f"file-{index}: leftover conflict marker" for index in range(12)]
        stdout = "\n".join(
            [conflicts[0], "file: trailing whitespace", *conflicts[1:]]
        )
        with mock.patch.object(
            stop_hook.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 2, stdout, "ignored"),
        ) as run:
            self.assertEqual(conflicts[:10], stop_hook.conflict_diagnostics())

        self.assertIn("diff", run.call_args.args[0])
        self.assertIn("--check", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
