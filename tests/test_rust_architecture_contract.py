import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WILDCARD_IMPORT_LINT = "#![cfg_attr(not(test), deny(clippy::wildcard_imports))]"


class RustArchitectureContractTest(unittest.TestCase):
    def test_flow_agent_runtime_uses_explicit_module_boundaries(self) -> None:
        source_root = (
            ROOT / "flow-agent" / "flow-agent-core" / "src" / "runtime"
        )
        production_sources = list(source_root.rglob("*.rs"))
        crate_root = (source_root.parent / "lib.rs").read_text(encoding="utf-8")
        self.assertIn(WILDCARD_IMPORT_LINT, crate_root)

        for path in production_sources:
            source = path.read_text(encoding="utf-8")
            relative = path.relative_to(ROOT)
            self.assertNotIn("include!(", source, str(relative))
            self.assertNotIn("use super::*", source, str(relative))
            self.assertNotRegex(source, r"pub use [^;]+::\*", str(relative))

        composition_root = (source_root / "mod.rs").read_text(encoding="utf-8")
        self.assertNotRegex(composition_root, r"pub use [^;]+::\*;")
        for module in [
            "apply",
            "config_io",
            "context",
            "context_persistence",
            "event_construction",
            "event_writer",
            "failures",
            "fixture_effects",
            "fixture_tools",
            "fs_guards",
            "live_events",
            "planning",
            "resume",
            "session",
            "session_authority",
            "session_bundle",
            "session_lock",
            "session_reading",
            "session_reservation",
            "tail",
            "types",
            "validate",
        ]:
            self.assertIn(f"mod {module};", composition_root)

        planning = (source_root / "planning.rs").read_text(encoding="utf-8")
        application = (source_root / "apply.rs").read_text(encoding="utf-8")
        for forbidden in [
            "RuntimeEventSink",
            "SerialSessionWriter",
            "apply_planned_fixture_effect",
        ]:
            self.assertNotIn(forbidden, planning)
        for forbidden in [
            "ResolvedRegistry",
            "load_flow_registry",
            "plan_flow_with_workspace",
        ]:
            self.assertNotIn(forbidden, application)

    def test_core_script_uses_explicit_rust_modules(self) -> None:
        source_root = ROOT / "core" / "core-script" / "src"
        production_sources = [
            path
            for path in source_root.rglob("*.rs")
            if path.name != "tests.rs"
        ]
        crate_root = (source_root / "lib.rs").read_text(encoding="utf-8")
        self.assertIn(WILDCARD_IMPORT_LINT, crate_root)

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

    def test_flow_agent_cli_uses_responsibility_modules(self) -> None:
        source_root = ROOT / "flow-agent" / "flow-agent-cli" / "src"
        main = (source_root / "main.rs").read_text(encoding="utf-8")
        self.assertIn(WILDCARD_IMPORT_LINT, main)

        for module in ["dispatch", "parsing", "streaming", "tail"]:
            self.assertIn(f"mod {module};", main)
            self.assertTrue((source_root / f"{module}.rs").is_file())
        for responsibility in [
            "fn dispatch(",
            "fn emit_mode(",
            "fn stream_live_operation(",
            "fn tail_command(",
        ]:
            self.assertNotIn(responsibility, main)
        self.assertLessEqual(len(main.splitlines()), 80)

        for path in source_root.glob("*.rs"):
            source = path.read_text(encoding="utf-8")
            relative = path.relative_to(ROOT)
            self.assertNotIn("include!(", source, str(relative))
            self.assertNotIn("use super::*", source, str(relative))
            self.assertNotRegex(source, r"pub use [^;]+::\*", str(relative))

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
