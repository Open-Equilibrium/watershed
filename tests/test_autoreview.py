import importlib.machinery
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
AUTOREVIEW = ROOT / ".agents" / "skills" / "autoreview" / "scripts" / "autoreview"
LOADER = importlib.machinery.SourceFileLoader("autoreview", str(AUTOREVIEW))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
assert SPEC is not None
autoreview = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(autoreview)


class AutoreviewTest(unittest.TestCase):
    def test_run_decodes_non_default_console_bytes(self) -> None:
        result = autoreview.run(
            [sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'\\x9d')"],
            ROOT,
        )

        self.assertEqual("\ufffd", result.stdout)


if __name__ == "__main__":
    unittest.main()
