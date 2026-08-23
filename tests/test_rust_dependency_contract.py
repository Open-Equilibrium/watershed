import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

from test_m1_validation_contract import tracked_validation_paths


ROOT = Path(__file__).resolve().parents[1]
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


def cargo_dependency_tables(
    manifest: object,
) -> list[tuple[tuple[str, ...], dict[str, object]]]:
    if not isinstance(manifest, dict):
        return []

    dependency_table_names = (
        "dependencies",
        "build-dependencies",
        "dev-dependencies",
    )
    tables: list[tuple[tuple[str, ...], dict[str, object]]] = []
    for name in dependency_table_names:
        table = manifest.get(name)
        if isinstance(table, dict):
            tables.append(((name,), table))

    workspace = manifest.get("workspace")
    if isinstance(workspace, dict):
        table = workspace.get("dependencies")
        if isinstance(table, dict):
            tables.append((("workspace", "dependencies"), table))

    targets = manifest.get("target")
    if isinstance(targets, dict):
        for selector, target in targets.items():
            if not isinstance(target, dict):
                continue
            for name in dependency_table_names:
                table = target.get(name)
                if isinstance(table, dict):
                    tables.append((("target", selector, name), table))
    return tables


def rust_product_manifest_paths(repo: Path) -> list[Path]:
    return [
        manifest_path
        for manifest_path in tracked_validation_paths(repo)
        if manifest_path.name == "Cargo.toml"
        and "target" not in manifest_path.relative_to(repo).parts
    ]


class RustDependencyContractTest(unittest.TestCase):
    def test_workspace_registry_dependencies_are_exactly_pinned(self) -> None:
        with (ROOT / "Cargo.toml").open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)

        violations: list[str] = []
        for dependency, specification in manifest["workspace"]["dependencies"].items():
            if isinstance(specification, dict):
                if "path" in specification:
                    continue
                version = specification.get("version")
            else:
                version = specification
            if not isinstance(version, str) or not version.startswith("="):
                violations.append(dependency)

        self.assertEqual(violations, [])

    def test_rust_product_manifests_have_no_node_runtime_dependency(self) -> None:
        violations: list[str] = []

        for manifest_path in rust_product_manifest_paths(ROOT):
            with manifest_path.open("rb") as manifest_file:
                manifest = tomllib.load(manifest_file)
            for table_path, table in cargo_dependency_tables(manifest):
                for dependency in forbidden_dependency_names(table):
                    violations.append(
                        f"{manifest_path.relative_to(ROOT)}:"
                        f"{'.'.join(table_path)}:{dependency}"
                    )

        self.assertEqual(violations, [])

    def test_rust_product_manifest_scan_excludes_untracked_protected_files(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo = Path(temporary_directory)
            subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
            tracked = repo / "Cargo.toml"
            tracked.write_text("[workspace]\n", encoding="utf-8")
            protected = repo / "credentials" / "Cargo.toml"
            protected.parent.mkdir()
            protected.write_text("[dependencies]\nnode = \"1\"\n", encoding="utf-8")
            subprocess.run(["git", "add", "--", tracked.name], cwd=repo, check=True)

            self.assertEqual(rust_product_manifest_paths(repo), [tracked])

    def test_cargo_dependency_tables_exclude_metadata(self) -> None:
        manifest = {
            "dependencies": {"node-api": "1"},
            "workspace": {
                "dependencies": {"nodejs": "1"},
                "metadata": {
                    "reporter": {"dependencies": {"node": "display only"}}
                },
            },
            "package": {
                "metadata": {
                    "reporter": {"dependencies": {"napi": "display only"}}
                }
            },
            "target": {
                "cfg(unix)": {
                    "build-dependencies": {"neon": "1"},
                    "metadata": {
                        "reporter": {
                            "dependencies": {"node_bridge": "display only"}
                        }
                    },
                }
            },
        }

        self.assertEqual(
            [path for path, _ in cargo_dependency_tables(manifest)],
            [
                ("dependencies",),
                ("workspace", "dependencies"),
                ("target", "cfg(unix)", "build-dependencies"),
            ],
        )

    def test_forbidden_dependency_names_checks_cargo_package_aliases(self) -> None:
        self.assertEqual(
            forbidden_dependency_names(
                {"node_bridge": {"package": "node-api", "version": "1"}}
            ),
            ["node-api"],
        )


if __name__ == "__main__":
    unittest.main()
