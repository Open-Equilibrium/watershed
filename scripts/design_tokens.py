"""Generates Liquid's Dart design tokens from the canonical docs/design/tokens.css (ADR-0194).

Run without arguments to rewrite the Dart file; `--check` fails when it is out of date.
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CSS = ROOT / "docs" / "design" / "tokens.css"
DART = ROOT / "liquid" / "app" / "lib" / "src" / "theme" / "tokens.g.dart"

BLOCK = re.compile(r"(:root|\.theme-light|\.theme-dark)\s*\{(.*?)\}", re.S)
DECLARATION = re.compile(r"--ws-([a-z0-9-]+)\s*:\s*([^;]+);")
COLOR = re.compile(r"#([0-9a-fA-F]{6})|rgba\(\s*(\d+),\s*(\d+),\s*(\d+),\s*([0-9.]+)\s*\)")
# dart format's line width; the output matches its layout so formatting never changes it.
WIDTH = 80
SHADOW = re.compile(r"(-?[\d.]+)(?:px)?\s+(-?[\d.]+)(?:px)?\s+([\d.]+)(?:px)?(?:\s+(-?[\d.]+)(?:px)?)?\s+(#[0-9a-fA-F]{6}|rgba\([^)]*\))")


def camel(name):
    head, *rest = name.split("-")
    return head + "".join(part.capitalize() for part in rest)


def number(text):
    value = float(text)
    return str(int(value)) if value.is_integer() else str(value)


def color(text, name):
    match = COLOR.fullmatch(text.strip())
    if not match:
        raise ValueError(f"--ws-{name}: unsupported color {text!r}")
    if match.group(1):
        return f"Color(0xFF{match.group(1).upper()})"
    red, green, blue, alpha = match.group(2, 3, 4, 5)
    # Round half up, as browsers do; round() would round 76.5 down to 76.
    opacity = int(float(alpha) * 255 + 0.5)
    return f"Color(0x{opacity:02X}{int(red):02X}{int(green):02X}{int(blue):02X})"


def wrapped(head, items, tail, indent):
    """One line if it fits, else dart format's one-item-per-line layout."""
    line = f"{indent}{head}{', '.join(items)}{tail}"
    if len(line) <= WIDTH:
        return [line]
    return [f"{indent}{head}"] + [f"{indent}  {item}," for item in items] + [f"{indent}{tail}"]


def shadows(text, name):
    parts = [part.strip() for part in re.split(r",\s*(?![^()]*\))", text)]
    rendered = []
    for part in parts:
        match = SHADOW.fullmatch(part)
        if not match:
            raise ValueError(f"--ws-{name}: unsupported shadow {part!r}")
        x, y, blur, spread, shade = match.groups()
        fields = [
            f"offset: Offset({number(x)}, {number(y)})",
            f"blurRadius: {number(blur)}",
            f"spreadRadius: {number(spread or '0')}",
            f"color: {color(shade, name)}",
        ]
        rendered += wrapped("BoxShadow(", fields, "),", "      ")
    return rendered


def unquoted(family):
    """A CSS font family without its matching surrounding quotes."""
    if len(family) >= 2 and family[0] == family[-1] and family[0] in "'\"":
        return family[1:-1]
    return family


def dart_string(text):
    """A single-quoted Dart literal of `text`."""
    escaped = text.replace("\\", "\\\\").replace("'", "\\'").replace("$", "\\$")
    return f"'{escaped}'"


def metric(name, value):
    """Returns the Dart type and expression of one theme-independent token."""
    if name.startswith("font-"):
        return "List<String>", [dart_string(unquoted(family.strip())) for family in value.split(",")]
    if name.startswith("fw-") and value.isdigit():
        return "FontWeight", f"FontWeight.w{value}"
    if match := re.fullmatch(r"([\d.]+)px", value):
        return "double", number(match.group(1))
    if match := re.fullmatch(r"(\d+)ms", value):
        return "Duration", f"Duration(milliseconds: {match.group(1)})"
    if match := re.fullmatch(r"cubic-bezier\(([^)]*)\)", value):
        return "Cubic", "Cubic(" + ", ".join(number(part) for part in match.group(1).split(",")) + ")"
    raise ValueError(f"--ws-{name}: unsupported value {value!r}")


def render(css):
    blocks = {selector: dict(DECLARATION.findall(body)) for selector, body in BLOCK.findall(css)}
    root, light, dark = (blocks.get(key, {}) for key in (":root", ".theme-light", ".theme-dark"))
    if light.keys() != dark.keys():
        raise ValueError("light and dark themes must define the same tokens")
    lines = [
        "// Generated from docs/design/tokens.css by scripts/design_tokens.py. Do not edit.",
        "",
        "import 'package:flutter/animation.dart';",
        "import 'package:flutter/painting.dart';",
        "",
        "/// Type, spacing, radii, rows and motion shared by both themes.",
        "abstract final class WsMetrics {",
    ]
    for name, value in root.items():
        kind, expression = metric(name, value.strip())
        head = f"static const {kind} {camel(name)} = "
        if isinstance(expression, list):
            lines += wrapped(head + "[", expression, "];", "  ")
        else:
            lines.append(f"  {head}{expression};")
    lines += ["}", "", "/// The colors and shadows of one theme.", "final class WsPalette {", "  const WsPalette({"]
    lines += [f"    required this.{camel(name)}," for name in light]
    lines.append("  });")
    lines.append("")
    for name in light:
        kind = "List<BoxShadow>" if name.startswith("shadow") else "Color"
        lines.append(f"  final {kind} {camel(name)};")
    for theme, values in (("light", light), ("dark", dark)):
        lines += ["", f"  static const {theme} = WsPalette("]
        for name, value in values.items():
            if name.startswith("shadow"):
                lines.append(f"    {camel(name)}: [")
                lines += shadows(value, name)
                lines.append("    ],")
            else:
                lines.append(f"    {camel(name)}: {color(value, name)},")
        lines.append("  );")
    lines += ["}", ""]
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--css", type=Path, default=CSS)
    parser.add_argument("--dart", type=Path, default=DART)
    parser.add_argument("--check", action="store_true", help="fail when the Dart file is out of date")
    args = parser.parse_args(argv)
    dart = render(args.css.read_text(encoding="utf-8"))
    if args.check:
        current = args.dart.read_text(encoding="utf-8") if args.dart.exists() else None
        if current != dart:
            print(f"{args.dart} is out of date; run scripts/design_tokens.py", file=sys.stderr)
            return 1
        return 0
    args.dart.write_text(dart, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
