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


def channels(css_color, backdrop=(0.0, 0.0, 0.0)):
    """A CSS color's channels from 0 to 1, drawn over an opaque backdrop when translucent."""
    match = TOKENS.COLOR.fullmatch(css_color.strip())
    if match.group(1):
        values, alpha = [int(match.group(1)[i:i + 2], 16) / 255 for i in (0, 2, 4)], 1.0
    else:
        values, alpha = [int(match.group(i)) / 255 for i in (2, 3, 4)], float(match.group(5))
    return [value * alpha + under * (1 - alpha) for value, under in zip(values, backdrop)]


def luminance(rgb):
    linear = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb]
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]


def contrast(first, second):
    light, dark = sorted([luminance(first), luminance(second)], reverse=True)
    return (light + 0.05) / (dark + 0.05)


def text_contrast(tokens, tone, background):
    """The contrast of a text tone on its background, both drawn over the canvas."""
    surface = channels(tokens[background], channels(tokens["canvas"]))
    return contrast(channels(tokens[tone], surface), surface)

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
        unclosed = SAMPLE + "\n/* :root { --ws-fs-ui: 99px; }"
        self.assertEqual(TOKENS.render(unclosed), TOKENS.render(SAMPLE),
                         "a comment left open runs to the end")
        quoted = TOKENS.render(SAMPLE.replace('"Segoe UI"', '"Segoe/*UI*/"'))
        self.assertIn("'Segoe/*UI*/'", quoted, "a quoted name keeps what looks like a comment")

    def test_themes_must_define_the_same_tokens(self):
        with self.assertRaisesRegex(ValueError, "light and dark"):
            TOKENS.render(SAMPLE.replace("--ws-text-muted: #9b9b9b;", ""))

    def test_unknown_values_fail_instead_of_guessing(self):
        with self.assertRaisesRegex(ValueError, "--ws-fs-ui"):
            TOKENS.render(SAMPLE.replace("14px", "1.4em"))

    def test_last_declaration_without_a_semicolon_is_kept(self):
        dart = TOKENS.render(SAMPLE.replace("cubic-bezier(0, 0, 0.2, 1);", "cubic-bezier(0, 0, 0.2, 1)"))
        self.assertIn("static const Cubic ease = Cubic(0, 0, 0.2, 1);", dart)

    def test_out_of_range_colors_are_rejected(self):
        for color in ["rgba(256, 0, 0, 1)", "rgba(0, 0, 0, 1.1)"]:
            with self.subTest(color=color), self.assertRaisesRegex(ValueError, "--ws-text-muted"):
                TOKENS.render(SAMPLE.replace("#787774", color))

    def test_repeated_selector_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "repeated selector"):
            TOKENS.render(SAMPLE + ":root { --ws-fs-ui: 5px; }")

    def test_quoted_font_family_keeps_its_comma(self):
        dart = TOKENS.render(SAMPLE.replace('ui-sans-serif, "Segoe UI", Arial', 'Arial, "ACME, Inc.", sans-serif'))
        self.assertIn("static const List<String> fontUi = ['Arial', 'ACME, Inc.', 'sans-serif'];", dart)

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

    def test_tag_text_meets_wcag_aa_on_every_tag_color(self):
        blocks = TOKENS.blocks(TOKENS.CSS.read_text())
        for theme in [".theme-light", ".theme-dark"]:
            tokens = blocks[theme]
            for index in range(10):
                with self.subTest(theme=theme, tag=index):
                    self.assertGreaterEqual(text_contrast(tokens, "tag-text", f"tag-{index}"), 4.5)

    def test_text_tones_meet_wcag_aa_wherever_they_carry_content(self):
        blocks = TOKENS.blocks(TOKENS.CSS.read_text())
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
                    self.assertGreaterEqual(text_contrast(tokens, tone, background), 4.5)

    def test_translucent_backgrounds_are_judged_over_the_canvas(self):
        # White text on a 30% white tint over a white canvas is white on white.
        tokens = {"canvas": "#ffffff", "tint": "rgba(255, 255, 255, 0.3)", "text": "#ffffff"}
        self.assertAlmostEqual(text_contrast(tokens, "text", "tint"), 1.0, places=2)

    def test_liquid_sets_font_sizes_only_through_tokens(self):
        raw = re.compile(r"fontSize:\s*[\d.]+")
        found = [f"{path.relative_to(ROOT)}: {match.group(0)}"
                 for path in (ROOT / "liquid" / "app" / "lib").rglob("*.dart")
                 for match in raw.finditer(path.read_text())]
        self.assertEqual(found, [])


if __name__ == "__main__":
    unittest.main()
