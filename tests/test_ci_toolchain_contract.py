import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
NODE_VERSION = "24.18.0"
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


def ci_push_branches(workflow: str) -> tuple[str, ...]:
    lines = workflow.splitlines()
    try:
        push_start = lines.index("  push:")
    except ValueError:
        raise AssertionError("CI push.branches must use the canonical block form")
    push_end = next(
        (
            index
            for index in range(push_start + 1, len(lines))
            if lines[index] and not lines[index].startswith("    ")
        ),
        len(lines),
    )
    try:
        branches_start = lines.index("    branches:", push_start + 1, push_end)
    except ValueError:
        raise AssertionError("CI push.branches must use the canonical block form")
    branches = []
    for line in lines[branches_start + 1 :]:
        if line and not line.startswith("      "):
            break
        item = line.strip()
        if not item.startswith("- "):
            continue
        branches.append(item[2:].strip().strip('"'))
    return tuple(branches)


class CiToolchainContractTest(unittest.TestCase):
    def test_tracked_file_listing_preserves_unusual_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo = Path(temporary_directory)
            subprocess.run(
                ["git", "init", "--quiet"], cwd=repo, check=True
            )
            tracked_paths = [
                "-leading.md",
                "unicodé.md",
                "ignored.txt",
            ]
            if os.name != "nt":
                tracked_paths.append("line\nbreak.md")
            for tracked_path in tracked_paths:
                blob = subprocess.run(
                    ["git", "hash-object", "-w", "--stdin"],
                    cwd=repo,
                    input=b"tracked\n",
                    capture_output=True,
                    check=True,
                ).stdout.decode("ascii").strip()
                subprocess.run(
                    [
                        "git",
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        f"100644,{blob},{tracked_path}",
                    ],
                    cwd=repo,
                    check=True,
                )

            result = subprocess.run(
                [
                    "node",
                    str(ROOT / "scripts" / "list-tracked-files.mjs"),
                    "*.md",
                ],
                cwd=repo,
                encoding="utf-8",
                capture_output=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertCountEqual(
            json.loads(result.stdout),
            [path for path in tracked_paths if path.endswith(".md")],
        )

    def test_tracked_file_listing_forwards_complete_git_failure(self) -> None:
        diagnostic = "git failure: " + ("x" * 1_000_000)
        with tempfile.TemporaryDirectory(
            prefix="watershed path with spaces "
        ) as temporary_directory:
            temporary_path = Path(temporary_directory)
            emitter = temporary_path / "emit_error.py"
            preload = temporary_path / "async_stderr.cjs"
            emitter.write_text(
                "import sys\n"
                f"sys.stderr.write({diagnostic!r})\n"
                "sys.exit(23)\n",
                encoding="utf-8",
            )
            preload.write_text(
                "if (process.env.WATERSHED_TEST_ASYNC_STDERR === '1') {\n"
                "  delete process.env.WATERSHED_TEST_ASYNC_STDERR;\n"
                "  const write = process.stderr.write.bind(process.stderr);\n"
                "  process.stderr.write = (...args) => {\n"
                "    setTimeout(() => write(...args), 10);\n"
                "    return false;\n"
                "  };\n"
                "}\n",
                encoding="utf-8",
            )
            if os.name == "nt":
                node_executable = shutil.which("node")
                self.assertIsNotNone(node_executable)
                fake_git = temporary_path / "git.exe"
                shutil.copy2(node_executable, fake_git)
                (temporary_path / "ls-files").write_text(
                    f"process.stderr.write({json.dumps(diagnostic)});\n"
                    "process.exit(23);\n",
                    encoding="utf-8",
                )
            else:
                fake_git = temporary_path / "git"
                fake_git.write_text(
                    "#!/bin/sh\n"
                    f"exec {shlex.quote(sys.executable)} "
                    f"{shlex.quote(str(emitter))}\n",
                    encoding="utf-8",
                )
                fake_git.chmod(0o755)
            environment = os.environ.copy()
            environment["PATH"] = (
                str(temporary_path)
                + os.pathsep
                + environment.get("PATH", "")
            )
            environment["NODE_OPTIONS"] = "--require=./async_stderr.cjs"
            environment["WATERSHED_TEST_ASYNC_STDERR"] = "1"

            result = subprocess.run(
                ["node", str(ROOT / "scripts" / "list-tracked-files.mjs")],
                cwd=temporary_path,
                env=environment,
                encoding="utf-8",
                capture_output=True,
            )
            if os.name == "nt":
                deadline = time.monotonic() + 5
                while True:
                    try:
                        fake_git.unlink()
                        break
                    except PermissionError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.01)

        self.assertEqual(result.returncode, 23)
        self.assertEqual(result.stderr, diagnostic)

    def test_node_and_pnpm_versions_are_pinned(self) -> None:
        self.assertEqual(
            (ROOT / ".node-version").read_text(encoding="utf-8"),
            f"{NODE_VERSION}\n",
        )
        package = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
        self.assertEqual(package["engines"]["node"], NODE_VERSION)
        self.assertEqual(package["engines"]["pnpm"], PNPM_VERSION)
        self.assertEqual(package["packageManager"], f"pnpm@{PNPM_VERSION}")

    def test_node_toolchain_docs_cover_every_documentation_gate(self) -> None:
        testing = (ROOT / "TESTING.md").read_text(encoding="utf-8")

        self.assertIn(
            "documentation gates (HTML rendering and link-manifest generation)",
            testing,
        )

    def test_ci_installs_and_verifies_the_pinned_node_toolchain(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assert_active_pinned_node_steps(workflow)
        self.assertIn("run: cargo fmt --all --check", workflow)
        self.assertIn(
            "$docsJson = node scripts/list-tracked-files.mjs '*.md' '*.html'",
            workflow,
        )
        self.assertIn("if ($LASTEXITCODE -ne 0) {", workflow)
        self.assertIn("$docs = @($docsJson | ConvertFrom-Json)", workflow)
        self.assertIn(
            "lychee --no-progress --include-fragments -- @docs", workflow
        )
        self.assertNotIn("git ls-files", workflow)

    def assert_active_pinned_node_steps(self, workflow: str) -> None:
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

        setup_index = workflow.index(setup_reference)
        node_step_index, node_commands = self.active_pwsh_step_commands(
            workflow, "Check Node version"
        )
        corepack_step_index, corepack_commands = self.active_pwsh_step_commands(
            workflow, "Enable Corepack"
        )
        self.assertEqual(node_commands[0], "if ((node --version) -ne 'v24.18.0') {")
        self.assertTrue(node_commands[1].startswith('throw "'))
        self.assertEqual(node_commands[2:], ["}"])
        self.assertEqual(corepack_commands[0], "corepack enable")
        self.assertEqual(
            corepack_commands[1], "if ((pnpm --version) -ne '11.15.1') {"
        )
        self.assertTrue(corepack_commands[2].startswith('throw "'))
        self.assertEqual(corepack_commands[3:], ["}"])
        self.assertLess(setup_index, node_step_index)
        self.assertLess(node_step_index, corepack_step_index)

    def active_pwsh_step_commands(
        self, workflow: str, step_name: str
    ) -> tuple[int, list[str]]:
        lines = workflow.splitlines()
        self.assertFalse(any(line.startswith("    if:") for line in lines))
        self.assertFalse(
            any(line.startswith("    continue-on-error:") for line in lines)
        )
        marker = f"      - name: {step_name}"
        step_start = lines.index(marker)
        step_end = next(
            (
                index
                for index in range(step_start + 1, len(lines))
                if lines[index].startswith("      - ")
            ),
            len(lines),
        )
        step_lines = lines[step_start:step_end]

        self.assertFalse(
            any(line.startswith("        if:") for line in step_lines)
        )
        self.assertFalse(
            any(line.startswith("        continue-on-error:") for line in step_lines)
        )
        self.assertEqual(
            [line for line in step_lines if line.startswith("        shell:")],
            ["        shell: pwsh"],
        )
        run_index = step_lines.index("        run: |")
        commands = [
            line.strip()
            for line in step_lines[run_index + 1 :]
            if line.startswith("          ")
            and line.strip()
            and not line.lstrip().startswith("#")
        ]
        return workflow.index(marker), commands

    def test_node_toolchain_contract_rejects_inert_commands(self) -> None:
        active = """      - name: Checkout
        uses: actions/checkout@1111111111111111111111111111111111111111 # v7.0.0

      - name: Install pinned Node
        uses: actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38 # v6.5.0
        with:
          node-version-file: .node-version

      - name: Check Node version
        shell: pwsh
        run: |
          if ((node --version) -ne 'v24.18.0') {
            throw "wrong Node"
          }

      - name: Enable Corepack
        shell: pwsh
        run: |
          corepack enable
          if ((pnpm --version) -ne '11.15.1') {
            throw "wrong pnpm"
          }
"""
        commented = active.replace("          corepack", "          # corepack").replace(
            "          if ((pnpm", "          # if ((pnpm"
        )
        commented_node = active.replace(
            "          if ((node", "          # if ((node"
        )
        disabled = active.replace(
            "      - name: Enable Corepack\n        shell: pwsh",
            "      - name: Enable Corepack\n"
            "        if: ${{ false }}\n"
            "        shell: pwsh",
        )
        wrong_shell = active.replace(
            "      - name: Enable Corepack\n        shell: pwsh",
            "      - name: Enable Corepack\n        shell: bash",
        )
        missing_shell = active.replace(
            "      - name: Enable Corepack\n        shell: pwsh\n",
            "      - name: Enable Corepack\n",
        )
        continue_on_error = active.replace(
            "      - name: Enable Corepack\n        shell: pwsh",
            "      - name: Enable Corepack\n"
            "        continue-on-error: true\n"
            "        shell: pwsh",
        )
        job_disabled = "jobs:\n  m1:\n    if: false\n    steps:\n" + active
        job_continue_on_error = (
            "jobs:\n  m1:\n    continue-on-error: true\n    steps:\n" + active
        )
        early_success = active.replace(
            "          corepack enable", "          exit 0\n          corepack enable"
        )

        for workflow in (
            commented,
            commented_node,
            disabled,
            wrong_shell,
            missing_shell,
            continue_on_error,
            job_disabled,
            job_continue_on_error,
            early_success,
        ):
            with self.subTest(workflow=workflow), self.assertRaises(
                (AssertionError, ValueError)
            ):
                self.assert_active_pinned_node_steps(workflow)

    def test_ci_runs_on_every_permitted_topic_branch(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assertEqual(
            ci_push_branches(workflow),
            ("main", *(f"{branch_type}/**" for branch_type in TOPIC_BRANCH_TYPES)),
        )

    def test_ci_branch_parser_ignores_comments_and_other_trigger_keys(self) -> None:
        workflow = """on:
  push:
    paths:
      - \"feat/**\"
    branches:
      # \"fix/**\"
      - main
"""

        self.assertEqual(ci_push_branches(workflow), ("main",))

    def test_rust_product_manifests_have_no_node_runtime_dependency(self) -> None:
        violations: list[str] = []

        for manifest_path in sorted(ROOT.rglob("Cargo.toml")):
            if "target" in manifest_path.parts:
                continue
            with manifest_path.open("rb") as manifest_file:
                manifest = tomllib.load(manifest_file)
            for table_path, table in cargo_dependency_tables(manifest):
                for dependency in forbidden_dependency_names(table):
                    violations.append(
                        f"{manifest_path.relative_to(ROOT)}:"
                        f"{'.'.join(table_path)}:{dependency}"
                    )

        self.assertEqual(violations, [])

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
