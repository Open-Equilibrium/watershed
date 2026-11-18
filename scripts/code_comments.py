import ast
import io
import re
import subprocess
import tokenize
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

NOT_CHECKED = re.compile(
    r"^\.(?:codex|agents)/"
    r"|(?:^|/)frb_generated[^/]*$"
    r"|(?:^|/)lib/src/rust/"
    r"|\.g\.dart$"
    r"|(?:^|/)GeneratedPluginRegistrant[^/]*$"
)


@dataclass(frozen=True)
class Syntax:
    token: re.Pattern[str]
    nested_blocks: bool = True
    interpolation: str = ""
    interpolated_quotes: str = ""


def _syntax(literal: str, other: str = "", brackets: str = "{}", **options: object) -> Syntax:
    alternatives = [
        r"(?P<line>//[^\n]*)",
        r"(?P<block>/\*)",
        literal,
        rf"(?P<open>{re.escape(brackets[0])})",
        rf"(?P<close>{re.escape(brackets[1])})",
    ]
    if other:
        alternatives.append(other)
    return Syntax(re.compile("|".join(alternatives)), **options)


RUST = _syntax(
    r'(?P<prefix>(?<!\w)[bc]?r(?=#*")|(?<!\w)[bc](?="))?(?P<hashes>#*)(?P<quote>")',
    other=r"'(?:\\.[^'\n]*|[^\\'\n])'",
)
DART = _syntax(
    r"""(?P<prefix>(?<![\w$])r)?(?P<hashes>)(?P<quote>'''|\"\"\"|'|")""",
    interpolation="${",
    interpolated_quotes="'\"",
)
JAVASCRIPT = _syntax(
    r"""(?P<prefix>)(?P<hashes>)(?P<quote>["'`])""",
    other=(
        r"(?:(?<=[(,=:\[!&|?{};\n])|(?<=[(,=:\[!&|?{};]\s)|(?<=return\s))"
        r"/(?![/*])(?:\\.|\[(?:\\.|[^\]\n])*\]|[^/\\\n\[])+/[a-z]*"
    ),
    nested_blocks=False,
    interpolation="${",
    interpolated_quotes="`",
)
SWIFT = _syntax(
    r'(?P<prefix>)(?P<hashes>#*)(?P<quote>"""|")',
    brackets="()",
    interpolation="\\(",
    interpolated_quotes='"',
)
C_LIKE = {
    ".c": RUST,
    ".rs": RUST,
    ".dart": DART,
    ".js": JAVASCRIPT,
    ".cjs": JAVASCRIPT,
    ".mjs": JAVASCRIPT,
    ".swift": SWIFT,
}
SHELL_TOKEN = re.compile(
    r"""'[^']*'"""
    r"""|\$'(?:[^'\\]|\\.)*'"""
    r'''|"(?:[^"\\]|\\.)*"'''
    r"""|\\."""
    r"""|<<<"""
    r"""|<<(?P<strip>-?)\s*(?P<delimiter_quote>['"]?)(?P<delimiter>[\w.-]+)(?P=delimiter_quote)"""
    r"""|(?P<comment>(?<![^\s;&|()])\#[^\n]*)""",
    re.DOTALL,
)


def _block_end(text: str, start: int, nested: bool) -> int:
    depth = 1
    for marker in re.compile(r"/\*|\*/" if nested else r"\*/").finditer(text, start + 2):
        depth += 1 if marker[0] == "/*" else -1
        if depth == 0:
            return marker.end()
    return len(text)


def _string_end(
    text: str, syntax: Syntax, literal: re.Match[str], found: list[tuple[int, str]]
) -> int:
    quote, hashes = literal["quote"], literal["hashes"]
    closing = quote + hashes
    if (literal["prefix"] or "").endswith("r"):
        end = text.find(closing, literal.end())
        return len(text) if end < 0 else end + len(closing)
    escape = "\\" + hashes
    opening = ""
    if quote[0] in syntax.interpolated_quotes:
        opening = syntax.interpolation.replace("\\", escape)
    parts = [part for part in (opening, escape, closing) if part]
    boundary = re.compile("|".join(re.escape(part) for part in parts))
    position = literal.end()
    while match := boundary.search(text, position):
        if match[0] == closing:
            return match.end()
        if match[0] == escape:
            position = match.end() + 1
        else:
            position = _scan_code(text, syntax, match.end(), found, True)
    return len(text)


def _scan_code(
    text: str, syntax: Syntax, position: int, found: list[tuple[int, str]], interpolated: bool
) -> int:
    depth = 0
    while match := syntax.token.search(text, position):
        position = match.end()
        if match["line"]:
            found.append((match.start(), match[0]))
        elif match["block"]:
            position = _block_end(text, match.start(), syntax.nested_blocks)
            found.append((match.start(), text[match.start() : position]))
        elif match["quote"]:
            position = _string_end(text, syntax, match, found)
        elif match["open"]:
            depth += 1
        elif match["close"]:
            if interpolated and depth == 0:
                return position
            depth -= 1
    return len(text)


def _shell_comments(text: str) -> list[tuple[int, str]]:
    found = []
    position = 0
    while match := SHELL_TOKEN.search(text, position):
        position = match.end()
        if match["comment"]:
            found.append((match.start(), match[0]))
        elif match["delimiter"]:
            indent = r"[ \t]*" if match["strip"] else ""
            end = re.compile(rf"^{indent}{re.escape(match['delimiter'])}$", re.MULTILINE)
            line_end = text.find("\n", position)
            closing = end.search(text, line_end + 1) if line_end >= 0 else None
            position = closing.end() if closing else len(text)
    return found


def _python_comments(text: str) -> list[tuple[int, str]]:
    lines = text.splitlines(keepends=True)
    starts = [0]
    for line in lines:
        starts.append(starts[-1] + len(line))
    found = [
        (starts[token.start[0] - 1] + token.start[1], token.string)
        for token in tokenize.generate_tokens(io.StringIO(text).readline)
        if token.type == tokenize.COMMENT
    ]
    scopes = (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)
    for node in ast.walk(ast.parse(text)):
        if isinstance(node, scopes) and ast.get_docstring(node, clean=False) is not None:
            first = node.body[0]
            source = ast.get_source_segment(text, first) or ""
            found.append((starts[first.lineno - 1] + first.col_offset, source))
    return sorted(found)


def find_comments(path: str, text: str) -> list[tuple[int, str]]:
    suffix = PurePosixPath(path).suffix
    found: list[tuple[int, str]] = []
    if suffix == ".py":
        found = _python_comments(text)
    elif suffix == ".sh":
        found = _shell_comments(text)
    elif suffix in C_LIKE:
        _scan_code(text, C_LIKE[suffix], 0, found, False)
    return [
        (text.count("\n", 0, offset) + 1, comment)
        for offset, comment in found
        if not (offset == 0 and comment.startswith("#!"))
    ]


def is_checked(path: str) -> bool:
    suffix = PurePosixPath(path).suffix
    return suffix in {*C_LIKE, ".py", ".sh"} and not NOT_CHECKED.search(path)


def repository_comments(root: Path) -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True, text=True
    ).stdout
    problems = []
    for path in filter(is_checked, listed.split("\0")):
        source = root / path
        if source.is_file():
            text = source.read_text(encoding="utf-8")
            for line, comment in find_comments(path, text):
                problems.append(f"{path}:{line}: {comment.splitlines()[0]}")
    return problems
