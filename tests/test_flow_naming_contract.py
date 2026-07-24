import re
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_m1_validation_contract import tracked_validation_paths


ROOT = Path(__file__).resolve().parents[1]
LEGACY_DOMAIN_WORD = bytes((108, 111, 111, 112)).decode("ascii")
LEGACY_DOMAIN_TYPE = LEGACY_DOMAIN_WORD.capitalize()
LEGACY_DOMAIN_CONSTANT = LEGACY_DOMAIN_WORD.upper()


def joined(*parts: str) -> str:
    return "".join(parts)


LEGACY_TEXT_PATTERNS = {
    joined(LEGACY_DOMAIN_TYPE, " Agent"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_TYPE, " Agent"))
    ),
    joined(LEGACY_DOMAIN_WORD, "-agent"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, "-agent"))
    ),
    joined(LEGACY_DOMAIN_WORD, "_agent"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, "_agent"))
    ),
    joined(LEGACY_DOMAIN_TYPE, "Agent"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_TYPE, "Agent"))
    ),
    joined("Sub", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("Sub", LEGACY_DOMAIN_WORD))
    ),
    joined("sub", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("sub", LEGACY_DOMAIN_WORD))
    ),
    joined("sub_", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("sub_", LEGACY_DOMAIN_WORD))
    ),
    joined("sub-", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("sub-", LEGACY_DOMAIN_WORD))
    ),
    joined(".", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined(".", LEGACY_DOMAIN_WORD))
    ),
    joined(LEGACY_DOMAIN_WORD, "-context-v0"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, "-context-v0"))
    ),
    joined(LEGACY_DOMAIN_WORD, "_id"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, "_id"))
    ),
    joined("parent_", LEGACY_DOMAIN_WORD, "_id"): re.compile(
        re.escape(joined("parent_", LEGACY_DOMAIN_WORD, "_id"))
    ),
    joined(LEGACY_DOMAIN_WORD, "_definition_id"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, "_definition_id"))
    ),
    joined("source_", LEGACY_DOMAIN_WORD, "_definition_id"): re.compile(
        re.escape(joined("source_", LEGACY_DOMAIN_WORD, "_definition_id"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".started"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".started"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".completed"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".completed"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".failed"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".failed"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".start"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".start"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".status"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".status"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".cancel"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".cancel"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".tail"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".tail"))
    ),
    joined(LEGACY_DOMAIN_WORD, ".export"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_WORD, ".export"))
    ),
    joined(LEGACY_DOMAIN_TYPE, "Block"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_TYPE, "Block"))
    ),
    joined(LEGACY_DOMAIN_TYPE, "Invocation"): re.compile(
        re.escape(joined(LEGACY_DOMAIN_TYPE, "Invocation"))
    ),
    joined("Resolved", LEGACY_DOMAIN_TYPE, "State"): re.compile(
        re.escape(joined("Resolved", LEGACY_DOMAIN_TYPE, "State"))
    ),
    joined("MAX_", LEGACY_DOMAIN_CONSTANT, "_"): re.compile(
        re.escape(joined("MAX_", LEGACY_DOMAIN_CONSTANT, "_"))
    ),
    joined("run_", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("run_", LEGACY_DOMAIN_WORD))
    ),
    joined("execute_", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("execute_", LEGACY_DOMAIN_WORD))
    ),
    joined("load_", LEGACY_DOMAIN_WORD, "_registry"): re.compile(
        re.escape(joined("load_", LEGACY_DOMAIN_WORD, "_registry"))
    ),
    joined("smoke-", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("smoke-", LEGACY_DOMAIN_WORD))
    ),
    joined("hello-", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("hello-", LEGACY_DOMAIN_WORD))
    ),
    joined("hello-sub", LEGACY_DOMAIN_WORD): re.compile(
        re.escape(joined("hello-sub", LEGACY_DOMAIN_WORD))
    ),
    joined("registry/", LEGACY_DOMAIN_WORD, "s/"): re.compile(
        re.escape(joined("registry/", LEGACY_DOMAIN_WORD, "s/"))
    ),
    joined("yaml:", LEGACY_DOMAIN_WORD): re.compile(
        rf"^{LEGACY_DOMAIN_WORD}:\s*$"
    ),
}


def tracked_paths(root: Path = ROOT) -> list[str]:
    return [
        path.relative_to(root).as_posix()
        for path in tracked_validation_paths(root)
    ]


class FlowNamingContractTest(unittest.TestCase):
    def test_tracked_paths_exclude_protected_files_and_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_root = Path(temp)
            root = temp_root / "repo"
            root.mkdir()
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            (root / "safe.txt").write_text("safe", encoding="utf-8")
            (root / ".env").write_text("protected sentinel", encoding="utf-8")
            tracked = ["safe.txt", ".env"]
            external = temp_root / "external.txt"
            external.write_text("external sentinel", encoding="utf-8")
            try:
                (root / "external-link.txt").symlink_to(external)
            except OSError:
                pass
            else:
                tracked.append("external-link.txt")
            subprocess.run(["git", "add", "--", *tracked], cwd=root, check=True)

            self.assertEqual(["safe.txt"], tracked_paths(root))

    def test_tracked_paths_and_text_use_flow_domain_vocabulary(self) -> None:
        violations: list[str] = []
        paths = tracked_paths()

        for relative_path in paths:
            normalized_path = relative_path.replace("\\", "/")
            path_parts = normalized_path.split("/")
            legacy_registry_directory = f"{LEGACY_DOMAIN_WORD}s"
            if legacy_registry_directory in path_parts:
                violations.append(f"{normalized_path}: legacy registry directory")
            for fixture_name in (
                joined("smoke-", LEGACY_DOMAIN_WORD),
                joined("hello-", LEGACY_DOMAIN_WORD),
                joined("hello-sub", LEGACY_DOMAIN_WORD),
            ):
                if fixture_name in normalized_path:
                    violations.append(
                        f"{normalized_path}: legacy fixture path {fixture_name}"
                    )

            path = ROOT / relative_path
            try:
                content = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for line_number, line in enumerate(content.splitlines(), start=1):
                for label, pattern in LEGACY_TEXT_PATTERNS.items():
                    if pattern.search(line):
                        violations.append(
                            f"{normalized_path}:{line_number}: {label}"
                        )

        self.assertEqual(violations, [])


if __name__ == "__main__":
    unittest.main()
