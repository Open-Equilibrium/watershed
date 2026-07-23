import subprocess
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEGACY_DOMAIN_WORD = bytes((108, 111, 111, 112)).decode("ascii")
LEGACY_DOMAIN_TYPE = LEGACY_DOMAIN_WORD.capitalize()


class M1ValidationContractTest(unittest.TestCase):
    def test_flow_agent_identity_has_no_stale_product_references(self) -> None:
        expected_paths = [
            ROOT / "flow-agent" / "flow-agent-core" / "Cargo.toml",
            ROOT / "flow-agent" / "flow-agent-cli" / "Cargo.toml",
            ROOT / "flow-agent" / "fixtures" / "smoke-flow" / ".flow" / "config.yaml",
            ROOT / "docs" / "concept" / "V-Spec_FlowAgent.html",
        ]
        self.assertEqual(
            [str(path.relative_to(ROOT)) for path in expected_paths if not path.is_file()],
            [],
        )

        stale_tokens = [
            LEGACY_DOMAIN_TYPE + " Agent",
            LEGACY_DOMAIN_TYPE + "Agent",
            LEGACY_DOMAIN_TYPE + "-Agent",
            LEGACY_DOMAIN_WORD + "-agent",
            LEGACY_DOMAIN_WORD + "_agent",
        ]
        excluded_parts = {".git", ".clawpatch", ".codex-logs", "node_modules", "target"}
        stale_references: dict[str, list[str]] = {}
        for path in ROOT.rglob("*"):
            if not path.is_file() or excluded_parts.intersection(path.parts):
                continue
            try:
                content = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            matches = [token for token in stale_tokens if token in content]
            if matches:
                stale_references[str(path.relative_to(ROOT))] = matches

        self.assertEqual(stale_references, {})
        cli_manifest = expected_paths[1].read_text(encoding="utf-8")
        self.assertIn('name = "flow"', cli_manifest)
        self.assertNotIn('name = "' + LEGACY_DOMAIN_WORD + '"', cli_manifest)

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

    def test_gitignore_keeps_flow_workspace_config_trackable(self) -> None:
        for path, ignored in [
            ("flow-agent/fixtures/new-fixture/.flow/config.yaml", False),
            ("flow-agent/fixtures/new-fixture/.flow/sessions/session.jsonl", True),
            ("flow-agent/fixtures/new-fixture/.flow/logs/session.log", True),
            ("flow-agent/fixtures/new-fixture/out/result.txt", True),
            ("docs/out/example.md", False),
        ]:
            with self.subTest(path=path):
                self.assert_git_ignore(path, ignored=ignored)


if __name__ == "__main__":
    unittest.main()
