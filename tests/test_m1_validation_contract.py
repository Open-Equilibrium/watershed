import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def coverage_ignore_regex(text: str) -> re.Pattern[str]:
    match = re.search(r"--ignore-filename-regex\s+'([^']+)'", text)
    if match is None:
        raise AssertionError("missing --ignore-filename-regex value")
    return re.compile(match.group(1))


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
        ignore = coverage_ignore_regex(workflow)
        for path in [
            "core/core-script/src/tests.rs",
            "core/core-policy/src/tests.rs",
            "proto/proto/src/tests.rs",
            "loop-agent/loop-agent-core/src/tests.rs",
            "loop-agent/loop-agent-core/tests/performance.rs",
        ]:
            self.assertRegex(path, ignore)
        for path in [
            "core/core-script/src/lib.rs",
            "core/core-policy/src/lib.rs",
            "proto/proto/src/lib.rs",
            "loop-agent/loop-agent-core/src/lib.rs",
        ]:
            self.assertNotRegex(path, ignore)
        self.assertIn("--show-missing-lines", workflow)
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", workflow)

    def test_crate_tests_are_external_to_production_libs(self) -> None:
        for path in [
            ROOT / "core" / "core-script" / "src" / "lib.rs",
            ROOT / "core" / "core-policy" / "src" / "lib.rs",
            ROOT / "proto" / "proto" / "src" / "lib.rs",
            ROOT / "loop-agent" / "loop-agent-core" / "src" / "lib.rs",
        ]:
            source = path.read_text(encoding="utf-8").replace("\r\n", "\n")
            self.assertIn("#[cfg(test)]\nmod tests;", source, path)
            self.assertNotRegex(source, r"#\[cfg\(test\)\]\s*mod tests\s*\{")
            self.assertNotRegex(source, r"#\[cfg\(test\)\]\s*fn ")

    def test_pr_template_lists_m1_coverage_gate(self) -> None:
        template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "cargo llvm-cov nextest --locked --workspace --fail-under-lines 95",
            template,
        )
        self.assertIn("--ignore-filename-regex", template)
        ignore = coverage_ignore_regex(template)
        self.assertRegex("core/core-script/src/tests.rs", ignore)
        self.assertNotRegex("core/core-script/src/lib.rs", ignore)
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

        self.assertRegex(plan, r"Updated: 2026-\d{2}-\d{2}")
        self.assertIn("**Status:** M1 implementation is in progress.", plan)
        self.assertIn("M1 Loop Agent implementation is active", plan)
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

    def test_hardening_surfaces_remain_present(self) -> None:
        sources = {
            "core_script": (ROOT / "core" / "core-script" / "src" / "lib.rs").read_text(
                encoding="utf-8"
            ),
            "loop_agent_core": (
                ROOT / "loop-agent" / "loop-agent-core" / "src" / "lib.rs"
            ).read_text(encoding="utf-8"),
        }

        for source_key, token in [
            ("core_script", "fn visit_loop"),
            ("loop_agent_core", "persist_reserved_session_prefix"),
            ("loop_agent_core", "verify_resume_definition_metadata"),
            ("loop_agent_core", "ensure_session_log_growth_within_limit"),
            ("loop_agent_core", "validate_appended_session_log_text"),
            ("loop_agent_core", "ensure_real_workspace_write_path"),
            ("loop_agent_core", "active_session_lock_message"),
        ]:
            self.assertIn(token, sources[source_key])

    def test_ci_trigger_scope_and_branch_protection_decision_are_recorded(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        decisions = (ROOT / "docs" / "decisions" / "open-decisions.html").read_text(
            encoding="utf-8"
        )
        plan = (ROOT / "PLAN.md").read_text(encoding="utf-8")

        self.assertIn("pull_request:\n    branches: [main]", workflow)
        self.assertIn("push:\n    branches: [main]", workflow)
        for token in [
            'id="d-056"',
            "D-056 - CI Trigger And Branch Protection Scope",
            "CI currently runs only for pull requests targeting main and pushes to main",
            "changing triggers",
        ]:
            self.assertIn(token, decisions)
        self.assertIn(
            "CI currently runs only for PRs targeting `main` and pushes to `main`; "
            "branch protection/ruleset activation remains D-056.",
            plan,
        )


if __name__ == "__main__":
    unittest.main()
