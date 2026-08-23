import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


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


class PythonLauncherTest(unittest.TestCase):
    def assert_launcher_module_test(self, body: str) -> None:
        result = run_node_module_test(
            f"""
            import assert from "node:assert/strict";
            import {{ runPython }} from {json.dumps((ROOT / "scripts" / "run-python.mjs").as_uri())};

            {body}
            """
        )

        self.assertEqual("", result.stdout)
        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)

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
        self.assert_launcher_module_test(
            f"""
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
            assert.deepEqual(fallbackCalls, ["python3", "python", "python"]);

            const failureCalls = [];
            const failureStatus = runPython(["script.py"], {{
              platform: "darwin",
              spawnSync(executable, args) {{
                failureCalls.push(executable);
                return args[0] === "-c" ? {{ status: 0 }} : {{ status: 7 }};
              }},
            }});
            assert.equal(failureStatus, 7);
            assert.deepEqual(failureCalls, ["python3", "python3"]);

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
            assert.equal(stderr.text, "missing Python 3 interpreter: tried python3, python\\n");
            """
        )

    def test_python_launcher_requires_python_three_for_fallbacks(self) -> None:
        self.assert_launcher_module_test(
            f"""
            const missing = () => ({{
              error: Object.assign(new Error("missing"), {{ code: "ENOENT" }}),
            }});
            const rejectedCalls = [];
            const stderr = {{ text: "", write(chunk) {{ this.text += chunk; }} }};
            const rejected = runPython(["script.py"], {{
              platform: "linux",
              stderr,
              spawnSync(executable, args) {{
                rejectedCalls.push({{ executable, args }});
                if (executable === "python3") return missing();
                if (executable === "python" && args[0] === "-c") return {{ status: 1 }};
                throw new Error("incompatible interpreter executed target arguments");
              }},
            }});
            assert.equal(rejected, 127);
            assert.deepEqual(rejectedCalls.map((call) => call.executable), ["python3", "python"]);
            assert.match(stderr.text, /missing Python 3 interpreter/);

            const compatibleCalls = [];
            const compatible = runPython(["script.py"], {{
              platform: "darwin",
              spawnSync(executable, args) {{
                compatibleCalls.push({{ executable, args }});
                return args[0] === "-c" ? {{ status: 0 }} : {{ status: 7 }};
              }},
            }});
            assert.equal(compatible, 7);
            assert.equal(compatibleCalls.length, 2);
            assert.equal(compatibleCalls[0].args[0], "-c");
            assert.deepEqual(compatibleCalls[1].args, ["script.py"]);
            """
        )

    def test_windows_python_launcher_failure_falls_back_to_python(self) -> None:
        self.assert_launcher_module_test(
            f"""
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
            assert.deepEqual(calls.map((call) => call.executable), ["py", "python3", "python", "python"]);
            assert.deepEqual(calls[0].args.slice(0, 2), ["-3", "-c"]);
            """
        )

    def test_windows_python_launcher_stops_on_probe_error_or_signal(self) -> None:
        self.assert_launcher_module_test(
            f"""
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

    def test_windows_python_launcher_preserves_script_failure(self) -> None:
        self.assert_launcher_module_test(
            f"""
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


if __name__ == "__main__":
    unittest.main()
