import argparse
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

    def test_extra_prompt_reads_utf8_under_a_legacy_locale(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="watershed-autoreview-prompt-"
        ) as temp_dir:
            prompt_path = Path(temp_dir) / "prompt.txt"
            expected = "Review the \U0001f30a change"
            prompt_path.write_bytes(expected.encode("utf-8"))
            original_read_text = Path.read_text

            def legacy_read_text(
                path: Path, encoding: str | None = None, errors: str | None = None
            ) -> str:
                return original_read_text(
                    path, encoding=encoding or "cp1252", errors=errors
                )

            args = argparse.Namespace(prompt=[], prompt_file=[str(prompt_path)])
            with mock.patch.object(
                Path, "read_text", autospec=True, side_effect=legacy_read_text
            ):
                loaded = autoreview.load_extra_prompt(args)

            self.assertEqual(expected, loaded)

    def test_file_based_engines_write_utf8_prompts_under_a_legacy_locale(self) -> None:
        expected = "Review the \U0001f30a change"
        original_write_text = Path.write_text
        captured: list[str] = []

        def legacy_write_text(
            path: Path,
            data: str,
            encoding: str | None = None,
            errors: str | None = None,
            newline: str | None = None,
        ) -> int:
            return original_write_text(
                path,
                data,
                encoding=encoding or "cp1252",
                errors=errors,
                newline=newline,
            )

        def successful_engine(
            command: list[str], cwd: Path, **_kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            prompt_path = (
                Path(command[command.index("-f") + 1])
                if "-f" in command
                else cwd / "prompt.txt"
            )
            captured.append(prompt_path.read_text(encoding="utf-8"))
            return subprocess.CompletedProcess(command, 0, "{}", "")

        common = {
            "thinking": None,
            "stream_engine_output": False,
            "model": None,
            "tools": True,
            "web_search": False,
        }
        droid_args = argparse.Namespace(**common, droid_bin="droid")
        copilot_args = argparse.Namespace(**common, copilot_bin="copilot")
        with (
            mock.patch.object(
                Path, "write_text", autospec=True, side_effect=legacy_write_text
            ),
            mock.patch.object(
                autoreview, "resolve_command", side_effect=lambda command, _repo: command
            ),
            mock.patch.object(
                autoreview, "run_with_heartbeat", side_effect=successful_engine
            ),
        ):
            self.assertEqual("{}", autoreview.run_droid(droid_args, ROOT, expected))
            self.assertEqual("{}", autoreview.run_copilot(copilot_args, ROOT, expected))

        self.assertEqual([expected, expected], captured)

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
