import subprocess
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class M1ValidationContractTest(unittest.TestCase):
    def assert_active_pinned_rust_step(self, workflow: str, version: str) -> None:
        lines = workflow.splitlines()
        marker = "      - name: Select pinned Rust"
        start = lines.index(marker)
        end = next(
            (
                index
                for index in range(start + 1, len(lines))
                if lines[index].startswith("      - ")
            ),
            len(lines),
        )
        step = lines[start:end]
        self.assertFalse(any(line.startswith("        if:") for line in step))
        run = step.index("        run: |")
        script = "\n".join(line[10:] for line in step[run + 1 :])
        commands = script.splitlines()
        escaped_version = version.replace(".", r"\.")
        self.assertIn(f"rustup override set {version}", commands)
        self.assertIn(
            "if ((rustc --version) -notmatch "
            f"'^rustc {escaped_version} ') {{",
            commands,
        )

    def test_ci_uses_the_pinned_rust_toolchain(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        with (ROOT / "rust-toolchain.toml").open("rb") as toolchain_file:
            version = tomllib.load(toolchain_file)["toolchain"]["channel"]

        self.assert_active_pinned_rust_step(workflow, version)

    def test_ci_toolchain_contract_rejects_inert_commands(self) -> None:
        active = """      - name: Select pinned Rust
        shell: pwsh
        run: |
          rustup override set 1.97.1
          if ((rustc --version) -notmatch '^rustc 1\\.97\\.1 ') {
            throw "wrong version"
          }
"""
        commented = active.replace("          rustup", "          # rustup").replace(
            "          if ((rustc", "          # if ((rustc"
        )
        disabled = active.replace(
            "        shell: pwsh", "        if: ${{ false }}\n        shell: pwsh"
        )

        for workflow in (commented, disabled):
            with self.subTest(workflow=workflow), self.assertRaises(
                (AssertionError, ValueError)
            ):
                self.assert_active_pinned_rust_step(workflow, "1.97.1")

    def assert_git_ignore(self, path: str, *, ignored: bool) -> None:
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--quiet", path],
            cwd=ROOT,
        )
        if result.returncode not in (0, 1):
            raise AssertionError(
                f"git check-ignore failed for {path} with {result.returncode}"
            )
        self.assertEqual(result.returncode == 0, ignored, path)

    def test_gitignore_keeps_loop_workspace_config_trackable(self) -> None:
        for path, ignored in [
            ("loop-agent/fixtures/new-fixture/.loop/config.yaml", False),
            ("loop-agent/fixtures/new-fixture/.loop/sessions/session.jsonl", True),
            ("loop-agent/fixtures/new-fixture/.loop/logs/session.log", True),
            ("loop-agent/fixtures/new-fixture/out/result.txt", True),
            ("docs/out/example.md", False),
        ]:
            with self.subTest(path=path):
                self.assert_git_ignore(path, ignored=ignored)


if __name__ == "__main__":
    unittest.main()
