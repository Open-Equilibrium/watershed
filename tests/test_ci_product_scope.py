import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_ci_toolchain_contract import ROOT, step_lines, step_run, workflow_text


class CiProductScopeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.name", "Scope fixture")
        self.git("config", "user.email", "scope@example.invalid")
        self.git("config", "commit.gpgSign", "false")
        self.git("config", "core.autocrlf", "false")
        self.base = self.commit({"README.md": "Product documentation\n"})
        self.git("update-ref", "refs/remotes/origin/main", self.base)

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout.strip()

    def commit(self, files: dict[str, str]) -> str:
        for name, content in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Scope fixture")
        return self.git("rev-parse", "HEAD")

    def classify(self, event_name: str, event: dict, expected: bool | None) -> None:
        event_path = self.root / "event.json"
        output_path = self.root / "output"
        event_path.write_text(json.dumps(event), encoding="utf-8")
        output_path.write_text("", encoding="utf-8")
        result = subprocess.run(
            ["node", str(ROOT / "scripts" / "ci-product-scope.mjs")],
            cwd=self.repo,
            env={
                **os.environ,
                "GITHUB_EVENT_NAME": event_name,
                "GITHUB_EVENT_PATH": str(event_path),
                "GITHUB_OUTPUT": str(output_path),
            },
            capture_output=True,
            text=True,
        )
        if expected is None:
            self.assertEqual(result.returncode, 1)
            self.assertIn("Cannot compare CI product changes", result.stderr)
            self.assertEqual(output_path.read_text(encoding="utf-8"), "")
        else:
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                output_path.read_text(encoding="utf-8"),
                f"product={str(expected).lower()}\n",
            )

    def test_setup_only_changes_do_not_hide_product_changes_in_a_pull_request(self) -> None:
        setup = self.commit({
            "AGENTS.md": "Fixture instructions\n",
            ".agents/skills/fixture/SKILL.md": "Fixture skill\n",
            ".codex/agents/fixture.toml": "fixture = true\n",
        })
        for event_name, event in (
            ("push", {"before": self.base}),
            ("push", {"before": "0" * 40}),
            ("pull_request", {"pull_request": {"base": {"sha": self.base}}}),
        ):
            with self.subTest(event=event_name, payload=event):
                self.classify(event_name, event, False)
        self.classify("workflow_dispatch", {}, True)

        product = self.commit({
            "README.md": "Changed product documentation\n",
            ".codex/agents/fixture.toml": "fixture = false\n",
        })
        self.classify("push", {"before": setup}, True)
        self.classify("push", {"before": "0" * 40}, True)
        self.commit({"AGENTS.md": "Changed fixture instructions\n"})
        self.classify("push", {"before": product}, False)
        self.classify(
            "pull_request", {"pull_request": {"base": {"sha": self.base}}}, True
        )

    def test_moving_product_content_into_setup_still_requires_product_gates(self) -> None:
        (self.repo / ".agents").mkdir()
        self.git("mv", "README.md", ".agents/README.md")
        self.git("commit", "--quiet", "-m", "Move fixture")
        self.classify("push", {"before": self.base}, True)

    def test_setup_exclusions_do_not_cover_similarly_named_product_paths(self) -> None:
        before = self.base
        for name in ("docs/AGENTS.md", ".codex-tools/helper.mjs", ".agents.md"):
            with self.subTest(path=name):
                current = self.commit({name: "Product fixture\n"})
                self.classify("push", {"before": before}, True)
                before = current

    def test_unavailable_comparison_base_fails_without_a_skip_result(self) -> None:
        self.classify("push", {"before": "f" * 40}, None)

    def test_workflow_classifies_before_every_product_gate(self) -> None:
        workflow = workflow_text()
        self.assertIn("          fetch-depth: 0", step_lines(workflow, "Checkout"))
        self.assertEqual(
            step_run(workflow, "Classify product changes"),
            "node scripts/ci-product-scope.mjs",
        )
        self.assertIn("        id: scope", step_lines(workflow, "Classify product changes"))
        steps = [line.removeprefix("      - name: ") for line in workflow.splitlines()
                 if line.startswith("      - name: ")]
        self.assertEqual(steps[:3], ["Checkout", "Install pinned Node", "Classify product changes"])
        for name in steps[:3]:
            self.assertFalse(any(line.startswith("        if:")
                                 for line in step_lines(workflow, name)))
        for name in steps[3:]:
            with self.subTest(step=name):
                conditions = [line.removeprefix("        if: ")
                              for line in step_lines(workflow, name)
                              if line.startswith("        if:")]
                self.assertEqual(len(conditions), 1)
                self.assertRegex(
                    conditions[0], r"^steps\.scope\.outputs\.product == 'true'(?:$| && \()"
                )
