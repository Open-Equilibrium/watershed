import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class RustArchitectureContractTest(unittest.TestCase):
    def test_core_script_uses_explicit_rust_modules(self) -> None:
        source_root = ROOT / "core" / "core-script" / "src"
        production_sources = [
            path
            for path in source_root.rglob("*.rs")
            if path.name != "tests.rs"
        ]

        for path in production_sources:
            source = path.read_text(encoding="utf-8")
            relative = path.relative_to(ROOT)
            self.assertNotIn("include!(", source, str(relative))
            self.assertNotIn("use super::*", source, str(relative))
            self.assertNotRegex(source, r"pub use [^;]+::\*", str(relative))

        module_root = (source_root / "script" / "mod.rs").read_text(encoding="utf-8")
        for module in [
            "canonical",
            "load",
            "model",
            "naming",
            "parser",
            "paths",
            "registry",
            "semantics",
        ]:
            self.assertIn(f"mod {module};", module_root)

    def test_proto_is_the_only_event_payload_schema_owner(self) -> None:
        runtime_root = ROOT / "flow-agent" / "flow-agent-core" / "src" / "runtime"
        runtime_sources = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(runtime_root.rglob("*.rs"))
        )
        runtime_validation = (runtime_root / "validate.rs").read_text(encoding="utf-8")
        payload_boundary = runtime_validation.split(
            "pub fn validate_event_payload", maxsplit=1
        )[1].split("pub struct SessionAppendValidationState", maxsplit=1)[0]

        self.assertNotIn("struct PayloadValidator", runtime_sources)
        self.assertNotIn("impl PayloadValidator", runtime_sources)
        self.assertNotIn("payload.tool_kind must", runtime_sources)
        self.assertNotIn("payload.network_access must", runtime_sources)
        self.assertIn("event.validate_v0()", payload_boundary)
        self.assertNotIn("match event.event_type", payload_boundary)


if __name__ == "__main__":
    unittest.main()
