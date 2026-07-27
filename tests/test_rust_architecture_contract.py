import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class RustArchitectureContractTest(unittest.TestCase):
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
