import importlib.machinery
import importlib.util
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
AUTOREVIEW = ROOT / ".agents" / "skills" / "autoreview" / "scripts" / "autoreview"
LOADER = importlib.machinery.SourceFileLoader("autoreview", str(AUTOREVIEW))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
assert SPEC is not None
autoreview = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(autoreview)


class AutoreviewTest(unittest.TestCase):
    def test_run_decodes_non_default_console_bytes(self) -> None:
        result = autoreview.run(
            [sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'\\x9d')"],
            ROOT,
        )

        self.assertEqual("\ufffd", result.stdout)

    def test_local_bundle_does_not_follow_untracked_symlinks(self) -> None:
        with (
            tempfile.TemporaryDirectory(prefix="watershed-autoreview-repo-") as repo_dir,
            tempfile.TemporaryDirectory(
                prefix="watershed-autoreview-external-"
            ) as external_dir,
        ):
            repo = Path(repo_dir)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            sentinel = "external-autoreview-sentinel"
            external = Path(external_dir) / "secret.txt"
            external.write_text(sentinel, encoding="utf-8")
            link = repo / "untracked-link.txt"
            try:
                link.symlink_to(external)
            except OSError as exc:
                self.skipTest(f"symlink creation unavailable: {exc}")

            bundle = autoreview.local_bundle(repo)

            self.assertNotIn(sentinel, bundle)
            self.assertIn("## untracked-link.txt\n[non-regular file omitted]", bundle)

    def test_local_bundle_omits_non_regular_untracked_entries(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="watershed-autoreview-repo-"
        ) as repo_dir:
            repo = Path(repo_dir)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            sentinel = "simulated-reparse-sentinel"
            untracked = repo / "untracked-entry.txt"
            untracked.write_text(sentinel, encoding="utf-8")
            original_lstat = Path.lstat

            def simulated_link(path: Path) -> os.stat_result:
                if path == untracked:
                    return os.stat_result((stat.S_IFLNK | 0o777,) + (0,) * 9)
                return original_lstat(path)

            with mock.patch.object(Path, "lstat", autospec=True, side_effect=simulated_link):
                bundle = autoreview.local_bundle(repo)

            self.assertNotIn(sentinel, bundle)
            self.assertIn("## untracked-entry.txt\n[non-regular file omitted]", bundle)


if __name__ == "__main__":
    unittest.main()
