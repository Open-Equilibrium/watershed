import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEGACY_DOMAIN_WORD = bytes((108, 111, 111, 112)).decode("ascii")
LEGACY_DOMAIN_TYPE = LEGACY_DOMAIN_WORD.capitalize()
PROTECTED_SCAN_DIRECTORIES = {
    ".git",
    ".flow",
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".docker",
    ".kube",
    "credentials",
    "secrets",
}
PROTECTED_SCAN_FILES = {
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".git-credentials",
    "credentials.toml",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ecdsa_sk",
    "id_ed25519_sk",
}


def is_protected_validation_path(relative_path: Path) -> bool:
    parts = tuple(part.lower() for part in relative_path.parts)
    name = parts[-1]
    if PROTECTED_SCAN_DIRECTORIES.intersection(parts):
        return True
    if any(
        parts[index : index + 2] in ((".config", "gcloud"), (".config", "gh"))
        for index in range(len(parts) - 1)
    ):
        return True
    return (
        name in PROTECTED_SCAN_FILES
        or name == ".env"
        or name.startswith(".env.")
        or name.endswith((".env", ".local", ".pem", ".key", ".p12", ".pfx"))
    )


def tracked_validation_paths(repo: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repo,
        check=True,
        capture_output=True,
    )
    root = repo.resolve()
    paths: list[Path] = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative_path = Path(raw_path.decode("utf-8"))
        path = repo / relative_path
        if is_protected_validation_path(relative_path) or path.is_symlink():
            continue
        resolved = path.resolve()
        if resolved.is_relative_to(root) and resolved.is_file():
            paths.append(path)
    return paths


class M1ValidationContractTest(unittest.TestCase):
    def test_validation_scan_excludes_untracked_and_protected_files(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="watershed-m1-validation-"
        ) as temp_dir:
            repo = Path(temp_dir)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            safe = repo / "safe.txt"
            safe.write_text("tracked", encoding="utf-8")
            untracked = repo / "untracked.txt"
            untracked.write_text("untracked", encoding="utf-8")
            protected = repo / ".env"
            protected.write_text("protected", encoding="utf-8")
            credential = repo / "credentials" / "token.txt"
            credential.parent.mkdir()
            credential.write_text("credential", encoding="utf-8")
            subprocess.run(
                ["git", "add", "--", "safe.txt", ".env", "credentials/token.txt"],
                cwd=repo,
                check=True,
            )

            paths = tracked_validation_paths(repo)

            self.assertEqual(paths, [safe])

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
        stale_references: dict[str, list[str]] = {}
        for path in tracked_validation_paths(ROOT):
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
        self.assertEqual(
            [line for line in step if line.startswith("        shell:")],
            ["        shell: pwsh"],
        )
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
        wrong_shell = active.replace("        shell: pwsh", "        shell: bash")
        missing_shell = active.replace("        shell: pwsh\n", "")

        for workflow in (commented, disabled, wrong_shell, missing_shell):
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
