import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class M1ValidationContractTest(unittest.TestCase):
    def test_ci_enforces_m1_coverage_gate(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("M1 gates", workflow)
        self.assertIn(
            "cargo llvm-cov nextest --locked --workspace --fail-under-lines 95",
            workflow,
        )
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", workflow)

    def test_pr_template_lists_m1_coverage_gate(self) -> None:
        template = (ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "cargo llvm-cov nextest --locked --workspace --fail-under-lines 95",
            template,
        )
        self.assertNotIn("cargo llvm-cov nextest --locked --workspace --no-report", template)


if __name__ == "__main__":
    unittest.main()
