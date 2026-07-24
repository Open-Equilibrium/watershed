import json
import re
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
NODE_VERSION = "22.23.1"
PNPM_VERSION = "11.15.1"
SETUP_NODE_RELEASE = "v6.5.0"
SETUP_NODE_SHA = "249970729cb0ef3589644e2896645e5dc5ba9c38"
TOPIC_BRANCH_TYPES = ("feat", "fix", "docs", "test", "ci", "chore", "refactor")
FORBIDDEN_NODE_RUNTIME_DEPENDENCY = re.compile(
    r"^(?:node|nodejs|node-api|napi|neon)(?:[-_].*)?$"
)


def forbidden_dependency_names(table: dict[str, object]) -> list[str]:
    return [
        package_name
        for dependency, specification in table.items()
        for package_name in [
            specification.get("package", dependency)
            if isinstance(specification, dict)
            else dependency
        ]
        if isinstance(package_name, str)
        and FORBIDDEN_NODE_RUNTIME_DEPENDENCY.fullmatch(package_name)
    ]


class CiToolchainContractTest(unittest.TestCase):
    def test_node_and_pnpm_versions_are_pinned(self) -> None:
        self.assertEqual(
            (ROOT / ".node-version").read_text(encoding="utf-8"),
            f"{NODE_VERSION}\n",
        )
        package = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
        self.assertEqual(package["engines"]["node"], NODE_VERSION)
        self.assertEqual(package["engines"]["pnpm"], PNPM_VERSION)
        self.assertEqual(package["packageManager"], f"pnpm@{PNPM_VERSION}")

    def test_ci_installs_and_verifies_the_pinned_node_toolchain(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        setup_reference = (
            f"uses: actions/setup-node@{SETUP_NODE_SHA} # {SETUP_NODE_RELEASE}"
        )

        self.assertIn(setup_reference, workflow)
        self.assertRegex(
            workflow,
            r"uses: actions/checkout@[0-9a-f]{40} # v7\.0\.0\n\n"
            r"\s+- name: Install pinned Node\n\s+"
            rf"{re.escape(setup_reference)}\n\s+with:\n"
            r"\s+node-version-file: \.node-version(?:\n|$)",
        )
        self.assertNotRegex(workflow, r"actions/setup-node@(?![0-9a-f]{40}\b)")
        self.assertNotIn("check-latest:", workflow)
        self.assertIn(
            "if ((node --version) -ne 'v22.23.1') {",
            workflow,
        )
        self.assertIn(
            "if ((pnpm --version) -ne '11.15.1') {",
            workflow,
        )

        setup_index = workflow.index(setup_reference)
        corepack_index = workflow.index("corepack enable")
        pnpm_check_index = workflow.index("pnpm --version")
        self.assertLess(setup_index, corepack_index)
        self.assertLess(corepack_index, pnpm_check_index)

    def test_ci_runs_on_every_permitted_topic_branch(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        push_section = workflow.split("  push:\n", 1)[1].split("\n\n", 1)[0]

        for branch_type in TOPIC_BRANCH_TYPES:
            with self.subTest(branch_type=branch_type):
                self.assertIn(f'"{branch_type}/**"', push_section)

    def test_rust_product_manifests_have_no_node_runtime_dependency(self) -> None:
        violations: list[str] = []

        def dependency_tables(
            value: object, parents: tuple[str, ...] = ()
        ) -> list[tuple[tuple[str, ...], dict[str, object]]]:
            if not isinstance(value, dict):
                return []
            tables: list[tuple[tuple[str, ...], dict[str, object]]] = []
            for key, child in value.items():
                path = (*parents, key)
                if key in {
                    "dependencies",
                    "build-dependencies",
                    "dev-dependencies",
                } and isinstance(child, dict):
                    tables.append((path, child))
                tables.extend(dependency_tables(child, path))
            return tables

        for manifest_path in sorted(ROOT.rglob("Cargo.toml")):
            if "target" in manifest_path.parts:
                continue
            with manifest_path.open("rb") as manifest_file:
                manifest = tomllib.load(manifest_file)
            for table_path, table in dependency_tables(manifest):
                for dependency in forbidden_dependency_names(table):
                    violations.append(
                        f"{manifest_path.relative_to(ROOT)}:"
                        f"{'.'.join(table_path)}:{dependency}"
                    )

        self.assertEqual(violations, [])

    def test_forbidden_dependency_names_checks_cargo_package_aliases(self) -> None:
        self.assertEqual(
            forbidden_dependency_names(
                {"node_bridge": {"package": "node-api", "version": "1"}}
            ),
            ["node-api"],
        )


if __name__ == "__main__":
    unittest.main()
