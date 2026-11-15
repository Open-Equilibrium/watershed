import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "scripts" / "check-dart-coverage.mjs"


def record(path: str, lines_found: int, lines_hit: int) -> str:
    return f"SF:{path}\nLF:{lines_found}\nLH:{lines_hit}\nend_of_record\n"


class DartCoverageGateTest(unittest.TestCase):
    def run_gate(self, *records: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="dart coverage ") as temporary:
            lcov = Path(temporary) / "lcov.info"
            lcov.write_text("".join(records), encoding="utf-8")
            return subprocess.run(
                ["node", str(GATE), str(lcov)], capture_output=True, text=True, timeout=30
            )

    def test_line_coverage_boundary_is_the_eighty_eight_percent_minimum(self) -> None:
        for hit, passes in ((88, True), (87, False)):
            with self.subTest(hit=hit):
                result = self.run_gate(record("lib/app.dart", 100, hit))
                self.assertEqual(result.returncode == 0, passes, result.stderr)

    def test_generated_and_ffi_glue_are_not_measured(self) -> None:
        result = self.run_gate(
            record("lib/app.dart", 100, 90),
            record("lib/src/rust/api/git.dart", 500, 0),
            record("lib/src/rust/frb_generated.dart", 500, 0),
            record("lib/src/theme/tokens.g.dart", 500, 0),
            record("lib/src/session/backend.dart", 500, 0),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("90.00%", result.stdout)

    def test_glue_exclusion_matches_only_the_backend_file(self) -> None:
        result = self.run_gate(
            record("lib/app.dart", 100, 90),
            record("lib/src/session/session_state.dart", 100, 0),
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("45.00%", result.stdout)

    def test_malformed_records_fail_the_gate_without_a_measurement(self) -> None:
        malformed = {
            "invalid counter": "SF:lib/app.dart\nLF:invalid\nLH:0\nend_of_record\n",
            "oversized counter": f"SF:lib/app.dart\nLF:1{'0' * 400}\nLH:1{'0' * 399}\nend_of_record\n",
            "missing counter": "SF:lib/app.dart\nLF:100\nend_of_record\n",
            "hits above found": record("lib/app.dart", 100, 101),
            "truncated last record": record("lib/app.dart", 100, 90) + "SF:lib/other.dart\nLF:100\nLH:90\n",
            "record cut by the next file": "SF:lib/app.dart\nLF:100\nLH:99\nSF:lib/other.dart\nend_of_record\n",
        }
        for case, lcov in malformed.items():
            with self.subTest(case=case):
                result = self.run_gate(lcov)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_totals_beyond_the_exact_integer_range_fail(self) -> None:
        # Each record is exact, but the total 2**53 is not, so the ratio cannot be trusted.
        result = self.run_gate(
            record("lib/a.dart", 2**53 - 1, 2**53 - 1),
            record("lib/b.dart", 1, 0),
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertEqual(result.stdout, "")

    def test_no_measured_lines_fails(self) -> None:
        result = self.run_gate(record("lib/src/rust/api/git.dart", 500, 500))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no Dart lines", result.stderr)


if __name__ == "__main__":
    unittest.main()
