import os
import re
import stat
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
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
DECISION_REFERENCE_PATTERN = re.compile(r"\bD-([0-9]{3})\b")
DECISION_ANCHOR_PATTERN = re.compile(r'<article id="d-([0-9]{3})"')


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
    relative_paths: list[Path] = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        try:
            relative_paths.append(Path(raw_path.decode("utf-8")))
        except UnicodeDecodeError:
            continue
    tracked_paths = set(relative_paths)
    paths: list[Path] = []
    for relative_path in relative_paths:
        path = repo / relative_path
        if is_protected_validation_path(relative_path) or path.is_symlink():
            continue
        try:
            metadata = path.lstat()
        except OSError:
            continue
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            continue
        resolved = path.resolve()
        if not resolved.is_relative_to(root):
            continue
        resolved_relative = resolved.relative_to(root)
        if (
            resolved_relative in tracked_paths
            and not is_protected_validation_path(resolved_relative)
            and resolved.is_file()
        ):
            paths.append(path)
    return paths


class M1ValidationContractTest(unittest.TestCase):
    def test_documented_decision_references_resolve_to_live_entries(self) -> None:
        decisions = (ROOT / "docs" / "decisions" / "open-decisions.html").read_text(
            encoding="utf-8"
        )
        live_ids = set(DECISION_ANCHOR_PATTERN.findall(decisions))
        referenced_ids: set[str] = set()
        for path in tracked_validation_paths(ROOT):
            if path.suffix in {".md", ".html"}:
                referenced_ids.update(
                    DECISION_REFERENCE_PATTERN.findall(path.read_text(encoding="utf-8"))
                )

        self.assertTrue(live_ids)
        self.assertEqual(referenced_ids - live_ids, set())

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

    def test_validation_scan_skips_non_utf8_tracked_path_bytes(self) -> None:
        result = subprocess.CompletedProcess(
            ["git", "ls-files", "-z"],
            0,
            stdout=b"README.md\0non-utf8-\xff.txt\0",
        )

        with mock.patch("subprocess.run", return_value=result):
            paths = tracked_validation_paths(ROOT)

        self.assertEqual(paths, [ROOT / "README.md"])

    def test_validation_scan_excludes_external_hardlinks(self) -> None:
        with (
            tempfile.TemporaryDirectory() as repo_directory,
            tempfile.TemporaryDirectory() as external_directory,
        ):
            repo = Path(repo_directory)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            tracked = repo / "tracked.txt"
            tracked.write_text("staged placeholder", encoding="utf-8")
            subprocess.run(["git", "add", "--", tracked.name], cwd=repo, check=True)
            tracked.unlink()
            external = Path(external_directory) / "credential.txt"
            external.write_text("external credential", encoding="utf-8")
            try:
                os.link(external, tracked)
            except OSError as error:
                self.skipTest(f"hard-link creation unavailable: {error}")

            self.assertEqual(tracked_validation_paths(repo), [])

    def test_validation_scan_excludes_resolved_untracked_and_protected_paths(
        self,
    ) -> None:
        for label, target_relative in [
            ("protected", Path("credentials/Cargo.toml")),
            ("untracked", Path("scratch/Cargo.toml")),
        ]:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary_directory:
                repo = Path(temporary_directory)
                subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
                target = repo / target_relative
                target.parent.mkdir()
                target.write_text(label, encoding="utf-8")
                safe_parent = repo / "safe"
                safe_parent.mkdir()
                tracked = safe_parent / "Cargo.toml"
                tracked.write_text("tracked", encoding="utf-8")
                subprocess.run(
                    ["git", "add", "--", "safe/Cargo.toml"], cwd=repo, check=True
                )
                tracked.unlink()
                safe_parent.rmdir()
                try:
                    safe_parent.symlink_to(target.parent, target_is_directory=True)
                except OSError as error:
                    if os.name != "nt":
                        self.skipTest(f"directory symlink creation unavailable: {error}")
                    result = subprocess.run(
                        [
                            "cmd",
                            "/c",
                            "mklink",
                            "/J",
                            str(safe_parent),
                            str(target.parent),
                        ],
                        capture_output=True,
                    )
                    if result.returncode != 0:
                        self.skipTest("directory link creation unavailable")

                self.assertEqual(tracked_validation_paths(repo), [])

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
            ("flow-agent/fixtures/new-fixture/.flow/sessions/session.jsonl", False),
            ("flow-agent/fixtures/new-fixture/.flow/logs/session.log", False),
            ("flow-agent/fixtures/new-fixture/out/result.txt", True),
            ("docs/out/example.md", False),
        ]:
            with self.subTest(path=path):
                self.assert_git_ignore(path, ignored=ignored)


if __name__ == "__main__":
    unittest.main()
