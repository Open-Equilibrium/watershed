import argparse
import importlib.machinery
import importlib.util
import json
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
    @staticmethod
    def report(*, finding: bool = False) -> dict[str, object]:
        findings: list[dict[str, object]] = []
        if finding:
            findings.append(
                {
                    "title": "Actionable regression",
                    "body": "The changed behavior is incorrect.",
                    "priority": "P1",
                    "confidence": 0.9,
                    "category": "regression",
                    "code_location": {"file_path": "src/lib.rs", "line": 1},
                }
            )
        return {
            "findings": findings,
            "overall_correctness": (
                "patch is incorrect" if finding else "patch is correct"
            ),
            "overall_explanation": "Deterministic fixture result.",
            "overall_confidence": 0.9,
        }

    @staticmethod
    def make_executable(path: Path) -> None:
        path.write_text("fixture", encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

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

    def test_target_selection_covers_local_branch_commit_and_clean_main(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="watershed-autoreview-repo-"
        ) as repo_dir:
            repo = Path(repo_dir)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            subprocess.run(
                ["git", "config", "user.email", "autoreview@example.invalid"],
                cwd=repo,
                check=True,
            )
            subprocess.run(
                ["git", "config", "user.name", "Autoreview Test"],
                cwd=repo,
                check=True,
            )
            tracked = repo / "tracked.txt"
            tracked.write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "tracked.txt"], cwd=repo, check=True)
            subprocess.run(
                ["git", "commit", "--quiet", "-m", "test: add fixture"],
                cwd=repo,
                check=True,
            )

            subprocess.run(
                ["git", "switch", "--quiet", "-c", "review-topic"],
                cwd=repo,
                check=True,
            )
            self.assertEqual(
                ("branch", "origin/main"),
                autoreview.choose_target(repo, "auto", None),
            )
            self.assertEqual(
                ("branch", "custom-base"),
                autoreview.choose_target(repo, "branch", "custom-base"),
            )
            self.assertEqual(
                ("commit", None),
                autoreview.choose_target(repo, "commit", None),
            )

            tracked.write_text("changed\n", encoding="utf-8")
            self.assertEqual(
                ("local", None),
                autoreview.choose_target(repo, "auto", None),
            )
            self.assertEqual(
                ("local", None),
                autoreview.choose_target(repo, "uncommitted", None),
            )

            subprocess.run(
                ["git", "restore", "tracked.txt"], cwd=repo, check=True
            )
            subprocess.run(
                ["git", "switch", "--quiet", "-C", "main"],
                cwd=repo,
                check=True,
            )
            with self.assertRaisesRegex(SystemExit, "no review target"):
                autoreview.choose_target(repo, "auto", None)

    def test_command_resolution_excludes_reviewed_checkout(self) -> None:
        with (
            tempfile.TemporaryDirectory(
                prefix="watershed-autoreview-repo-"
            ) as repo_dir,
            tempfile.TemporaryDirectory(
                prefix="watershed-autoreview-tools-"
            ) as tools_dir,
        ):
            repo = Path(repo_dir)
            tools = Path(tools_dir)
            local_tool = repo / "review-engine"
            external_tool = tools / "review-engine"
            explicit_tool = repo / "trusted-engine"
            for path in (local_tool, external_tool, explicit_tool):
                self.make_executable(path)

            search_path = os.pathsep.join((str(repo), ".", str(tools)))
            with mock.patch.dict(os.environ, {"PATH": search_path}):
                self.assertEqual(
                    str(external_tool.resolve()),
                    autoreview.resolve_command("review-engine", repo),
                )
                self.assertEqual(
                    str(explicit_tool),
                    autoreview.resolve_command(f".{os.sep}trusted-engine", repo),
                )

    def test_reviewer_validates_structured_results_and_engine_failures(self) -> None:
        args = argparse.Namespace(engine="codex")
        for has_finding in (False, True):
            expected = self.report(finding=has_finding)
            with mock.patch.object(
                autoreview, "run_engine", return_value=json.dumps(expected)
            ):
                actual = autoreview.run_reviewer(
                    args, ROOT, "prompt", {"src/lib.rs"}, []
                )
            self.assertEqual(expected, actual)

        invalid = self.report()
        del invalid["overall_confidence"]
        with mock.patch.object(
            autoreview, "run_engine", return_value=json.dumps(invalid)
        ):
            with self.assertRaisesRegex(SystemExit, "missing required key"):
                autoreview.run_reviewer(args, ROOT, "prompt", {"src/lib.rs"}, [])

        with mock.patch.object(
            autoreview, "run_engine", side_effect=SystemExit("engine failed")
        ):
            with self.assertRaisesRegex(SystemExit, "engine failed"):
                autoreview.run_reviewer(args, ROOT, "prompt", {"src/lib.rs"}, [])

    def test_main_maps_clean_and_finding_reports_to_exit_codes(self) -> None:
        args = argparse.Namespace(
            mode="local",
            base=None,
            engine="codex",
            model=None,
            thinking=None,
            reviewers=None,
            panel=False,
            tools=True,
            web_search=False,
            commit="HEAD",
            dry_run=False,
            parallel_tests=None,
            prompt=None,
            prompt_file=None,
            dataset=None,
            json_output=None,
            output=None,
            require_finding=[],
            expect_findings=False,
        )
        dependencies = {
            "parse_args": mock.Mock(return_value=args),
            "reviewer_args": mock.Mock(return_value=[args]),
            "repo_root": mock.Mock(return_value=ROOT),
            "choose_target": mock.Mock(return_value=("local", None)),
            "current_branch": mock.Mock(return_value="topic"),
            "local_bundle": mock.Mock(return_value="bundle"),
            "build_prompt": mock.Mock(return_value="prompt"),
            "review_paths": mock.Mock(return_value={"src/lib.rs"}),
        }
        for expected_exit, report in (
            (0, self.report()),
            (1, self.report(finding=True)),
        ):
            dependencies["run_reviewer"] = mock.Mock(return_value=report)
            with mock.patch.dict(autoreview.main.__globals__, dependencies):
                self.assertEqual(expected_exit, autoreview.main())


if __name__ == "__main__":
    unittest.main()
