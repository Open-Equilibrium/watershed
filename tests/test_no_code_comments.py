import unittest
from pathlib import Path

from scripts import code_comments


ROOT = Path(__file__).resolve().parents[1]

CASES = {
    "rust": (
        "a.rs",
        """
        //! module
        /// item
        /* outer /* inner */ still */
        fn f<'a>(x: &'a str) { g("// s", r#"/* "// "#, b"//", '"', '\\''); } // end
        """,
        ["//! module", "/// item", "/* outer /* inner */ still */", "// end"],
    ),
    "c": ("a.c", 'int main(void) { puts("// s"); } /* end */', ["/* end */"]),
    "dart": (
        "a.dart",
        """
        /// item
        final a = '// s' "/* s" r'\\' '''// s
        ''' '${b['// key']}' '${c /* inside */}';
        // end
        """,
        ["/// item", "/* inside */", "// end"],
    ),
    "javascript": (
        "a.mjs",
        """
        const a = "// s" + '/* s';
        const b = `// ${c("// key")} /* s`;
        const d = e.replace(/["/]\\/\\//g, "");
        const i = (j) => /"/.test(j);
        const f = g / h; /* one /* */
        // end
        """,
        ["/* one /* */", "// end"],
    ),
    "swift": (
        "a.swift",
        """
        /// item
        let a = "// s \\(b["// key"]!)" + #"// "s \\(c)"#
        let d = \"\"\"
        // s
        \"\"\" /* one /* two */ */
        """,
        ["/// item", "/* one /* two */ */"],
    ),
    "python": (
        "a.py",
        '#!/usr/bin/env python3\n"""module"""\n'
        'def f():\n    """item"""\n    return "# s"  # end\n',
        ['"""module"""', '"""item"""', "# end"],
    ),
    "shell": (
        "a.sh",
        "#!/bin/sh\n"
        "# one\n"
        "echo a#b '# s' \"# s\" $# ${#x} $'#\\'' \\# # two\n"
        "cat <<EOF\n# body\nEOF\n"
        "cat <<-'END'\n\t# body\n\tEND\n"
        "cat <<< '# s'\n"
        "x=$((1 << 2))\n"
        "# three\n"
        "cat <<A <<B # four\n# body\nA\n# body\nB\n",
        ["# one", "# two", "# three", "# four"],
    ),
}


class CodeCommentsTest(unittest.TestCase):
    def test_finds_comments_and_nothing_inside_literals(self) -> None:
        for language, (path, source, expected) in CASES.items():
            with self.subTest(language):
                found = code_comments.find_comments(path, source)
                self.assertEqual([text for _, text in found], expected)

    def test_reports_the_line_of_each_comment(self) -> None:
        found = code_comments.find_comments("a.rs", "fn f() {}\n\n// here\n")
        self.assertEqual(found, [(3, "// here")])

    def test_generated_and_maintainer_owned_files_are_not_checked(self) -> None:
        for path in (
            ".codex/hooks/guard.py",
            ".agents/skills/x/run.py",
            "liquid/liquid-bridge/src/frb_generated.rs",
            "liquid/app/lib/src/rust/api/session.dart",
            "liquid/app/lib/src/theme/tokens.g.dart",
            "liquid/app/macos/Flutter/GeneratedPluginRegistrant.swift",
            "README.md",
        ):
            with self.subTest(path):
                self.assertFalse(code_comments.is_checked(path))
        self.assertTrue(code_comments.is_checked("liquid/app/lib/main.dart"))

    def test_the_repository_has_no_code_comments(self) -> None:
        self.assertEqual(code_comments.repository_comments(ROOT), [])


if __name__ == "__main__":
    unittest.main()
