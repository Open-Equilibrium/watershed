import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class M1ValidationContractTest(unittest.TestCase):
    def test_ci_enforces_m1_coverage_gate(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("M1 gates", workflow)
        for token in [
            "cargo llvm-cov nextest",
            "--locked",
            "--workspace",
            "--fail-under-lines 95",
        ]:
            self.assertIn(token, workflow)
        self.assertIn("--ignore-filename-regex", workflow)
        self.assertIn(r"(^|[\\/])(tests?|src[\\/]tests\\.rs)([\\/]|$)", workflow)
        self.assertIn("--show-missing-lines", workflow)
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", workflow)

    def test_pr_template_lists_m1_coverage_gate(self) -> None:
        template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "cargo llvm-cov nextest --locked --workspace --fail-under-lines 95",
            template,
        )
        self.assertIn("--ignore-filename-regex", template)
        self.assertIn(r"(^|[\\/])(tests?|src[\\/]tests\\.rs)([\\/]|$)", template)
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", template)

    def test_security_docs_do_not_overstate_m1_enforcement(self) -> None:
        security = (ROOT / "SECURITY.md").read_text(encoding="utf-8")

        self.assertNotIn("M1 enforces it deterministically in-process", security)
        self.assertIn("deterministic in-process execution/emulation", security)

    def test_security_docs_do_not_advertise_json_schema_engine(self) -> None:
        security = (ROOT / "SECURITY.md").read_text(encoding="utf-8")

        self.assertNotIn("JSON Schema validation", security)
        self.assertIn("strict parser validation", security)

    def test_plan_tracks_active_m1_implementation(self) -> None:
        plan = (ROOT / "PLAN.md").read_text(encoding="utf-8")

        self.assertIn("Updated: 2026-07-03", plan)
        self.assertIn("**Status:** M1 implementation is in progress.", plan)
        self.assertIn(
            "`2026-07-03` — M1 Loop Agent implementation is active", plan
        )
        self.assertNotIn("## Ordered follow-up steps to start M1 with Codex", plan)

    def test_readme_documents_loop_agent_quickstart_and_layout(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8")

        for token in [
            "## Build and run Loop Agent",
            "cargo run -p loop-agent-cli -- run smoke-loop --emit jsonl",
            ".loop/config.yaml",
            "registry_root: registry",
            "registry/{tools,instructions,phases,loops,connections}/",
            ".loop/sessions/<session_id>.jsonl",
            ".loop/logs/<session_id>.log",
            "out/",
        ]:
            self.assertIn(token, readme)


if __name__ == "__main__":
    unittest.main()
