import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from test_ci_toolchain_contract import step_run, workflow_text


ROOT = Path(__file__).resolve().parents[1]


class TrackedFileContractTest(unittest.TestCase):
    def test_tracked_file_listing_preserves_unusual_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo = Path(temporary_directory)
            subprocess.run(
                ["git", "init", "--quiet"], cwd=repo, check=True
            )
            tracked_paths = [
                "-leading.md",
                "unicodé.md",
                "ignored.txt",
                "AGENTS.md",
                ".agents/skills/fixture/SKILL.md",
                ".codex/agents/fixture.md",
                "docs/AGENTS.md",
                "docs/page.html",
            ]
            if os.name != "nt":
                tracked_paths.append("line\nbreak.md")
            for tracked_path in tracked_paths:
                blob = subprocess.run(
                    ["git", "hash-object", "-w", "--stdin"],
                    cwd=repo,
                    input=b"tracked\n",
                    capture_output=True,
                    check=True,
                ).stdout.decode("ascii").strip()
                subprocess.run(
                    [
                        "git",
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        f"100644,{blob},{tracked_path}",
                    ],
                    cwd=repo,
                    check=True,
                )

            result = subprocess.run(
                [
                    "node",
                    str(ROOT / "scripts" / "list-tracked-files.mjs"),
                    "*.md",
                ],
                cwd=repo,
                encoding="utf-8",
                capture_output=True,
            )
            link_step = step_run(workflow_text(), "Check documentation links")
            manifest_line = next(
                line for line in link_step.splitlines()
                if line.startswith("$docsJson = node ")
            )
            manifest_args = shlex.split(manifest_line.removeprefix("$docsJson = node "))
            manifest_args[0] = str(ROOT / manifest_args[0])
            product_docs = subprocess.run(
                ["node", *manifest_args], cwd=repo, encoding="utf-8", capture_output=True
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertCountEqual(
            json.loads(result.stdout),
            [path for path in tracked_paths if path.endswith(".md")],
        )
        self.assertEqual(product_docs.returncode, 0, product_docs.stderr)
        self.assertCountEqual(
            json.loads(product_docs.stdout),
            ["-leading.md", "unicodé.md", "docs/AGENTS.md", "docs/page.html"]
            + (["line\nbreak.md"] if os.name != "nt" else []),
        )

    def test_tracked_file_listing_forwards_complete_git_failure(self) -> None:
        diagnostic = "git failure: " + ("x" * 1_000_000)
        with tempfile.TemporaryDirectory(
            prefix="watershed path with spaces "
        ) as temporary_directory:
            temporary_path = Path(temporary_directory)
            emitter = temporary_path / "emit_error.py"
            preload = temporary_path / "async_stderr.cjs"
            emitter.write_text(
                "import sys\n"
                f"sys.stderr.write({diagnostic!r})\n"
                "sys.exit(23)\n",
                encoding="utf-8",
            )
            preload.write_text(
                "if (process.env.WATERSHED_TEST_ASYNC_STDERR === '1') {\n"
                "  delete process.env.WATERSHED_TEST_ASYNC_STDERR;\n"
                "  const write = process.stderr.write.bind(process.stderr);\n"
                "  process.stderr.write = (...args) => {\n"
                "    setTimeout(() => write(...args), 10);\n"
                "    return false;\n"
                "  };\n"
                "}\n",
                encoding="utf-8",
            )
            if os.name == "nt":
                node_executable = shutil.which("node")
                self.assertIsNotNone(node_executable)
                fake_git = temporary_path / "git.exe"
                shutil.copy2(node_executable, fake_git)
                (temporary_path / "ls-files").write_text(
                    f"process.stderr.write({json.dumps(diagnostic)});\n"
                    "process.exit(23);\n",
                    encoding="utf-8",
                )
            else:
                fake_git = temporary_path / "git"
                fake_git.write_text(
                    "#!/bin/sh\n"
                    f"exec {shlex.quote(sys.executable)} "
                    f"{shlex.quote(str(emitter))}\n",
                    encoding="utf-8",
                )
                fake_git.chmod(0o755)
            environment = os.environ.copy()
            environment["PATH"] = (
                str(temporary_path)
                + os.pathsep
                + environment.get("PATH", "")
            )
            environment["NODE_OPTIONS"] = "--require=./async_stderr.cjs"
            environment["WATERSHED_TEST_ASYNC_STDERR"] = "1"

            result = subprocess.run(
                ["node", str(ROOT / "scripts" / "list-tracked-files.mjs")],
                cwd=temporary_path,
                env=environment,
                encoding="utf-8",
                capture_output=True,
            )
            if os.name == "nt":
                deadline = time.monotonic() + 5
                while True:
                    try:
                        fake_git.unlink()
                        break
                    except PermissionError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.01)

        self.assertEqual(result.returncode, 23)
        self.assertEqual(result.stderr, diagnostic)


if __name__ == "__main__":
    unittest.main()
