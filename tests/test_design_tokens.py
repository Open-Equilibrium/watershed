"""Liquid's Dart design tokens must follow the canonical CSS tokens."""

import importlib.util
import re
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("design_tokens", ROOT / "scripts" / "design_tokens.py")
TOKENS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TOKENS)

SAMPLE = """
/* comment */
:root {
  --ws-font-ui: ui-sans-serif, "Segoe UI", Arial;
  --ws-fs-ui: 14px;
  --ws-fw-bold: 700;
  --ws-motion-fast: 120ms;
  --ws-ease: cubic-bezier(0, 0, 0.2, 1);
}
.theme-light {
  --ws-text-muted: #787774;
  --ws-shadow: 0 0 0 1px rgba(15, 15, 15, 0.05), 0 3px 6px rgba(15, 15, 15, 0.1);
}
.theme-dark {
  --ws-text-muted: #9b9b9b;
  --ws-shadow: 0 3px 6px rgba(0, 0, 0, 0.3);
}
"""


class DesignTokens(unittest.TestCase):
    def test_every_value_kind_becomes_a_dart_constant(self):
        dart = TOKENS.render(SAMPLE)
        for expected in [
            "static const List<String> fontUi = ['ui-sans-serif', 'Segoe UI', 'Arial'];",
            "static const double fsUi = 14;",
            "static const FontWeight fwBold = FontWeight.w700;",
            "static const Duration motionFast = Duration(milliseconds: 120);",
            "static const Cubic ease = Cubic(0, 0, 0.2, 1);",
            "textMuted: Color(0xFF787774),",
            "      BoxShadow(\n        offset: Offset(0, 3),\n        blurRadius: 6,\n"
            "        spreadRadius: 0,\n        color: Color(0x4D000000),\n      ),",
            "        spreadRadius: 1,\n        color: Color(0x0D0F0F0F),",
            "final Color textMuted;",
            "final List<BoxShadow> shadow;",
        ]:
            self.assertIn(expected, dart)

    def test_font_families_become_valid_dart_strings(self):
        dart = TOKENS.render(SAMPLE.replace(
            'ui-sans-serif, "Segoe UI", Arial', """'Inter', "Noto's Sans", Font$Name"""))
        self.assertIn(
            r"static const List<String> fontUi = ['Inter', 'Noto\'s Sans', 'Font\$Name'];", dart)

    def test_comments_define_no_tokens(self):
        commented = SAMPLE.replace("--ws-fs-ui: 14px;", "--ws-fs-ui: 14px; /* --ws-fs-ui: 99px; } */")
        commented += "/* .theme-dark { --ws-text-muted: #000000; } */\n"
        self.assertEqual(TOKENS.render(commented), TOKENS.render(SAMPLE))

    def test_themes_must_define_the_same_tokens(self):
        with self.assertRaisesRegex(ValueError, "light and dark"):
            TOKENS.render(SAMPLE.replace("--ws-text-muted: #9b9b9b;", ""))

    def test_unknown_values_fail_instead_of_guessing(self):
        with self.assertRaisesRegex(ValueError, "--ws-fs-ui"):
            TOKENS.render(SAMPLE.replace("14px", "1.4em"))

    def test_check_reports_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            css, dart = Path(temporary) / "tokens.css", Path(temporary) / "tokens.g.dart"
            css.write_text(SAMPLE)
            self.assertEqual(TOKENS.main(["--css", str(css), "--dart", str(dart)]), 0)
            self.assertEqual(TOKENS.main(["--css", str(css), "--dart", str(dart), "--check"]), 0)
            css.write_text(SAMPLE.replace("14px", "15px"))
            self.assertEqual(TOKENS.main(["--css", str(css), "--dart", str(dart), "--check"]), 1)

    def test_the_committed_dart_tokens_match_the_canonical_css(self):
        self.assertEqual(TOKENS.main(["--check"]), 0)

    def test_text_tones_meet_wcag_aa_wherever_they_carry_content(self):
        blocks = TOKENS.blocks(TOKENS.CSS.read_text())

        def luminance(hex_color):
            channels = [int(hex_color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
            linear = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
            return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]

        def contrast(first, second):
            light, dark = sorted([luminance(first), luminance(second)], reverse=True)
            return (light + 0.05) / (dark + 0.05)

        backgrounds = ["canvas", "subtle", "surface", "hover", "selected", "titlebar", "code",
                       "warning-subtle"]
        pairs = [(tone, background) for tone in ["text", "text-muted", "text-faint"]
                 for background in backgrounds]
        # Diff line numbers sit on the strong diff colors in muted.
        pairs += [("text-muted", "diff-add-strong"), ("text-muted", "diff-del-strong")]
        # Status colors are text on surfaces, hover rows and their own tints.
        for tone, tint in [("accent", "accent-subtle"), ("success", "success-subtle"),
                           ("warning", "warning-subtle"), ("danger", "danger-subtle")]:
            pairs += [(tone, background)
                      for background in ["canvas", "subtle", "surface", "hover", tint]]
        pairs += [("on-accent", "accent")]
        pairs += [("on-data", f"data-{lane}") for lane in (1, 2, 3)]
        pairs += [("syn-com", background) for background in ["diff-add", "diff-del", "code"]]
        for theme in [".theme-light", ".theme-dark"]:
            tokens = blocks[theme]
            for tone, background in pairs:
                with self.subTest(theme=theme, tone=tone, background=background):
                    self.assertGreaterEqual(
                        contrast(tokens[tone], tokens[background]), 4.5)

    def test_liquid_sets_font_sizes_only_through_tokens(self):
        raw = re.compile(r"fontSize:\s*[\d.]+")
        found = [f"{path.relative_to(ROOT)}: {match.group(0)}"
                 for path in (ROOT / "liquid" / "app" / "lib").rglob("*.dart")
                 for match in raw.finditer(path.read_text())]
        self.assertEqual(found, [])


if __name__ == "__main__":
    unittest.main()
