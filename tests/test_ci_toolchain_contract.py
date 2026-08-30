import json
import re
import shlex
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PACKAGE = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
NODE_VERSION = (ROOT / ".node-version").read_text(encoding="utf-8").strip()
PNPM_VERSION = PACKAGE["packageManager"].removeprefix("pnpm@")
CHECKOUT_RELEASE = "v7.0.1"
CHECKOUT_SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1"
SETUP_NODE_RELEASE = "v7.0.0"
SETUP_NODE_SHA = "820762786026740c76f36085b0efc47a31fe5020"
INSTALL_ACTION_RELEASE = "v2.87.2"
INSTALL_ACTION_SHA = "1ed6d7be6168f6c9046541087ff549b6bc581fdf"
GATE_TOOLS = (
    "cargo-nextest@0.9.143,cargo-llvm-cov@0.9.0,cargo-audit@0.22.2,"
    "cargo-deny@0.20.2,lychee@0.24.2"
)
M11_BUDGET_FEATURE = "m11-budget-evidence"
M11_BUDGET_EXAMPLE = "m11_budgets"
M12_STARTUP_FEATURE = "m12-startup-evidence"
M12_STARTUP_EXAMPLE = "m12_executor_startup"
TEST_ISOLATION_CARGO_CONFIG = (
    'target."cfg(all())".runner = ["node", "../../scripts/run-isolated-rust-test.mjs"]'
)
UPLOAD_ARTIFACT_RELEASE = "v7.0.1"
UPLOAD_ARTIFACT_SHA = "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
TOPIC_BRANCH_TYPES = ("feat", "fix", "docs", "test", "ci", "chore", "refactor")


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



class CiWorkflowContractTest(unittest.TestCase):
    def test_ci_uses_the_pinned_rust_toolchain(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        with (ROOT / "rust-toolchain.toml").open("rb") as toolchain_file:
            version = tomllib.load(toolchain_file)["toolchain"]["channel"]

        self.assert_active_pinned_rust_step(workflow, version)

    def test_rust_toolchain_contract_rejects_pin_tampering(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        with (ROOT / "rust-toolchain.toml").open("rb") as toolchain_file:
            version = tomllib.load(toolchain_file)["toolchain"]["channel"]
        tampered = workflow.replace(
            "open('rust-toolchain.toml', 'rb')",
            "open('another-toolchain.toml', 'rb')",
        )

        with self.assertRaises(AssertionError):
            self.assert_active_pinned_rust_step(tampered, version)

    def test_node_and_pnpm_versions_are_pinned(self) -> None:
        self.assertEqual(
            (ROOT / ".node-version").read_text(encoding="utf-8"),
            f"{NODE_VERSION}\n",
        )
        self.assertEqual(PACKAGE["packageManager"], f"pnpm@{PNPM_VERSION}")

    def test_node_toolchain_docs_cover_every_documentation_gate(self) -> None:
        testing = (ROOT / "TESTING.md").read_text(encoding="utf-8")

        self.assertIn(
            "documentation gates (HTML rendering and link-manifest generation)",
            testing,
        )
        self.assertIn("the Node advisory audit", testing)
        self.assertIn("the Rust test-isolation runner", testing)
        self.assertIn("`pnpm audit --lockfile-only`", testing)

    def test_ci_installs_and_verifies_the_pinned_node_toolchain(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assert_active_pinned_node_steps(workflow)
        self.assert_all_remote_actions_pinned(workflow)
        self.assertNotIn("git ls-files", workflow)

    def test_ci_collects_and_covers_feature_gated_reporter(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        manifest = tomllib.loads(
            (ROOT / "flow-agent" / "flow-agent-core" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        reporter = next(
            example
            for example in manifest["example"]
            if example["name"] == M11_BUDGET_EXAMPLE
        )
        self.assertEqual(reporter["required-features"], [M11_BUDGET_FEATURE])
        startup_reporter = next(
            example
            for example in manifest["example"]
            if example["name"] == M12_STARTUP_EXAMPLE
        )
        self.assertEqual(
            startup_reporter["required-features"], [M12_STARTUP_FEATURE]
        )

        self.assert_active_ci_gates(workflow)

    def test_ci_gate_contract_rejects_mandatory_gate_removal(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        mutations = {
            "formatting disabled": workflow.replace(
                "      - name: Check formatting\n"
                "        shell: pwsh\n",
                "      - name: Check formatting\n"
                "        if: false\n"
                "        shell: pwsh\n",
            ),
            "clippy removed": workflow.replace(
                "      - name: Check lints\n"
                "        run: cargo clippy --locked --workspace --all-targets --all-features -- -D warnings\n\n",
                "",
            ),
            "coverage threshold": workflow.replace(
                "          --fail-under-lines 90\n", ""
            ),
            "M1.1 evidence execution": workflow.replace(
                "          cargo run --locked -p flow-agent-core --release \\\n",
                "          true \\\n",
            ),
            "M1.1 evidence upload tolerated": workflow.replace(
                "          if-no-files-found: error\n",
                "          if-no-files-found: error\n"
                "        continue-on-error: true\n",
            ),
            "M1.2 startup baseline execution": workflow.replace(
                "            --features m12-startup-evidence --example m12_executor_startup \\\n",
                "            --features m12-startup-evidence --example m11_budgets \\\n",
            ),
            "M1.2 startup baseline enforcement tolerated": workflow.replace(
                "      - name: Enforce M1.2 direct-runner startup baseline\n"
                "        if: matrix.os == 'ubuntu-24.04' && steps.m12_startup_baseline.outcome != 'success'\n"
                "        shell: bash\n",
                "      - name: Enforce M1.2 direct-runner startup baseline\n"
                "        if: matrix.os == 'ubuntu-24.04' && steps.m12_startup_baseline.outcome != 'success'\n"
                "        continue-on-error: true\n"
                "        shell: bash\n",
            ),
            "RustSec audit removed": workflow.replace(
                "      - name: Check RustSec advisories\n"
                "        run: cargo audit\n\n",
                "",
            ),
            "dependency policy removed": workflow.replace(
                "      - name: Check dependency policy\n"
                "        run: cargo deny check\n\n",
                "",
            ),
            "HTML rendering removed": workflow.replace(
                "      - name: Render HTML docs\n"
                "        run: pnpm run docs:render-check\n\n",
                "",
            ),
            "documentation links disabled": workflow.replace(
                "      - name: Check documentation links\n"
                "        shell: pwsh\n",
                "      - name: Check documentation links\n"
                "        if: false\n"
                "        shell: pwsh\n",
            ),
        }

        for label, mutated in mutations.items():
            with self.subTest(label=label):
                self.assertNotEqual(workflow, mutated)
                with self.assertRaises(AssertionError):
                    self.assert_active_ci_gates(mutated)

    def assert_active_ci_gates(self, workflow: str) -> None:
        single_line_gates = {
            "Check formatting": "cargo fmt --all --check",
            "Check lints": (
                "cargo clippy --locked --workspace --all-targets "
                "--all-features -- -D warnings"
            ),
            "Check RustSec advisories": "cargo audit",
            "Check dependency policy": "cargo deny check",
            "Check Node advisories": "pnpm audit --lockfile-only",
            "Render HTML docs": "pnpm run docs:render-check",
        }
        for step_name, command in single_line_gates.items():
            self.assert_active_single_line_step(workflow, step_name, command)

        self.assertEqual(
            self.active_folded_step_tokens(workflow, "Run tests"),
            [
                "cargo",
                "nextest",
                "run",
                "--config",
                TEST_ISOLATION_CARGO_CONFIG,
                "--locked",
                "--workspace",
                "--all-targets",
                "--all-features",
            ],
        )
        self.assertEqual(
            self.active_folded_step_tokens(workflow, "Run Rustdoc tests"),
            [
                "cargo",
                "--config",
                ".cargo/test-isolation.toml",
                "test",
                "--locked",
                "--workspace",
                "--all-features",
                "--doc",
            ],
        )
        self.assertEqual(
            self.step_lines(workflow, "Run M1.1 performance evidence")[1],
            [
                "      - name: Run M1.1 performance evidence",
                "        id: m11_evidence",
                "        if: matrix.os == 'ubuntu-24.04'",
                "        continue-on-error: true",
                "        shell: bash",
                "        run: |",
                "          mkdir -p target/m11-performance",
                "          cargo run --locked -p flow-agent-core --release \\",
                "            --features m11-budget-evidence --example m11_budgets \\",
                "            > target/m11-performance/m11-performance-evidence.jsonl",
                "",
            ],
        )
        self.assert_active_pinned_gate_actions(workflow)
        self.assertEqual(
            self.step_lines(workflow, "Enforce M1.1 evidence integrity")[1],
            [
                "      - name: Enforce M1.1 evidence integrity",
                "        if: matrix.os == 'ubuntu-24.04' && steps.m11_evidence.outcome != 'success'",
                "        shell: bash",
                "        run: exit 1",
                "",
            ],
        )
        self.assertEqual(
            self.step_lines(workflow, "Run M1.2 direct-runner startup baseline")[1],
            [
                "      - name: Run M1.2 direct-runner startup baseline",
                "        id: m12_startup_baseline",
                "        if: matrix.os == 'ubuntu-24.04'",
                "        continue-on-error: true",
                "        shell: bash",
                "        run: |",
                "          mkdir -p target/m12-startup",
                "          cargo run --locked -p flow-agent-core --release \\",
                "            --features m12-startup-evidence --example m12_executor_startup \\",
                "            > target/m12-startup/m12-direct-runner-baseline.jsonl",
                "",
            ],
        )
        self.assertEqual(
            self.step_lines(workflow, "Upload M1.2 direct-runner startup baseline")[1],
            [
                "      - name: Upload M1.2 direct-runner startup baseline",
                "        if: matrix.os == 'ubuntu-24.04' && always()",
                f"        uses: actions/upload-artifact@{UPLOAD_ARTIFACT_SHA} # {UPLOAD_ARTIFACT_RELEASE}",
                "        with:",
                "          name: m12-direct-runner-startup-baseline",
                "          path: target/m12-startup/m12-direct-runner-baseline.jsonl",
                "          if-no-files-found: error",
                "",
            ],
        )
        self.assertEqual(
            self.step_lines(workflow, "Enforce M1.2 direct-runner startup baseline")[1],
            [
                "      - name: Enforce M1.2 direct-runner startup baseline",
                "        if: matrix.os == 'ubuntu-24.04' && steps.m12_startup_baseline.outcome != 'success'",
                "        shell: bash",
                "        run: exit 1",
                "",
            ],
        )
        self.assertEqual(
            self.active_folded_step_tokens(workflow, "Check line coverage"),
            [
                "cargo",
                "llvm-cov",
                "nextest",
                "--config",
                TEST_ISOLATION_CARGO_CONFIG,
                "--locked",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--fail-under-lines",
                "90",
                "--ignore-filename-regex",
                r"((^|[\\/])(tests?|src[\\/]tests\.rs)([\\/]|$)|flow-agent[\\/]flow-agent-cli[\\/]src[\\/](main|parsing)\.rs$|flow-agent[\\/]flow-agent-core[\\/]src[\\/]runtime[\\/](m11_budget_evidence|m12_startup_evidence)(\.rs|[\\/]))",
                "--show-missing-lines",
            ],
        )
        _, link_commands = self.active_pwsh_step_commands(
            workflow, "Check documentation links"
        )
        self.assertEqual(
            link_commands,
            [
                "$docsJson = node scripts/list-tracked-files.mjs '*.md' '*.html'",
                "if ($LASTEXITCODE -ne 0) {",
                "exit $LASTEXITCODE",
                "}",
                "$docs = @($docsJson | ConvertFrom-Json)",
                "lychee --no-progress --include-fragments -- @docs",
            ],
        )

    def test_ci_runs_public_surface_rustdoc_tests(self) -> None:
        testing_contract = (ROOT / "TESTING.md").read_text(encoding="utf-8")
        mandatory_gates = next(
            line
            for line in testing_contract.splitlines()
            if line.startswith("- Mandatory gates:")
        )
        self.assertIn(
            "`cargo --config .cargo/test-isolation.toml test --locked --workspace --all-features --doc`",
            mandatory_gates,
        )

    def assert_active_pinned_node_steps(self, workflow: str) -> None:
        setup_reference = (
            f"uses: actions/setup-node@{SETUP_NODE_SHA} # {SETUP_NODE_RELEASE}"
        )

        self.assertIn(setup_reference, workflow)
        self.assertRegex(
            workflow,
            rf"uses: actions/checkout@{CHECKOUT_SHA} # {CHECKOUT_RELEASE}\n\n"
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
        self.assertEqual(
            node_commands,
            [
                "$expectedNode = (Get-Content -Raw .node-version).Trim()",
                'if ((node --version) -ne "v$expectedNode") {',
                'throw "Node $expectedNode must be active for CI gates"',
                "}",
            ],
        )
        self.assertEqual(corepack_commands[0], "corepack enable")
        self.assertEqual(
            corepack_commands[1],
            "$packageManager = (Get-Content -Raw package.json | ConvertFrom-Json).packageManager",
        )
        self.assertEqual(corepack_commands[2], "$expectedPnpm = $packageManager -replace '^pnpm@', ''")
        self.assertEqual(corepack_commands[3], "if ((pnpm --version) -ne $expectedPnpm) {")
        self.assertEqual(corepack_commands[4], 'throw "pnpm $expectedPnpm must be active for CI gates"')
        self.assertEqual(corepack_commands[5:], ["}"])
        self.assertLess(setup_index, node_step_index)
        self.assertLess(node_step_index, corepack_step_index)

    def assert_active_pinned_rust_step(self, workflow: str, version: str) -> None:
        _, commands = self.active_pwsh_step_commands(workflow, "Select pinned Rust")
        self.assertNotIn(version, workflow)
        self.assertEqual(
            commands,
            [
                '$toolchain = node scripts/run-python.mjs -c "import tomllib; '
                "print(tomllib.load(open('rust-toolchain.toml', 'rb'))"
                "['toolchain']['channel'])\"",
                "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
                "$escapedToolchain = [regex]::Escape($toolchain)",
                'if ((rustc --version) -notmatch "^rustc $escapedToolchain ") {',
                'throw "Rust $toolchain must be active for CI gates"',
                "}",
            ],
        )

    def assert_active_pinned_gate_actions(self, workflow: str) -> None:
        self.assertEqual(
            self.step_lines(workflow, "Install gate tools")[1],
            [
                "      - name: Install gate tools",
                f"        uses: taiki-e/install-action@{INSTALL_ACTION_SHA} # {INSTALL_ACTION_RELEASE}",
                "        with:",
                f"          tool: {GATE_TOOLS}",
                "",
            ],
        )
        self.assertEqual(
            self.step_lines(workflow, "Upload M1.1 performance evidence")[1],
            [
                "      - name: Upload M1.1 performance evidence",
                "        if: matrix.os == 'ubuntu-24.04' && always()",
                f"        uses: actions/upload-artifact@{UPLOAD_ARTIFACT_SHA} # {UPLOAD_ARTIFACT_RELEASE}",
                "        with:",
                "          name: m11-performance-evidence",
                "          path: target/m11-performance/m11-performance-evidence.jsonl",
                "          if-no-files-found: error",
                "",
            ],
        )

    def assert_all_remote_actions_pinned(self, workflow: str) -> None:
        for line in workflow.splitlines():
            match = re.match(r"^\s*(?:-\s+)?uses:\s*([^\s#]+)", line)
            if match is None:
                continue
            reference = match.group(1)
            if reference.startswith("./"):
                continue
            self.assertRegex(
                reference,
                r"^[^@/\s]+/[^@\s]+@[0-9a-f]{40}$",
                f"remote action must use a full commit SHA: {reference}",
            )

    def active_pwsh_step_commands(
        self, workflow: str, step_name: str
    ) -> tuple[int, list[str]]:
        step_index, step_lines = self.active_step_lines(workflow, step_name)
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
        return step_index, commands

    def assert_active_single_line_step(
        self, workflow: str, step_name: str, command: str
    ) -> None:
        _, step_lines = self.active_step_lines(workflow, step_name)
        self.assertEqual(
            [line for line in step_lines if line.startswith("        run:")],
            [f"        run: {command}"],
        )

    def active_step_lines(self, workflow: str, step_name: str) -> tuple[int, list[str]]:
        lines = workflow.splitlines()
        self.assertFalse(any(line.startswith("    if:") for line in lines))
        self.assertFalse(
            any(line.startswith("    continue-on-error:") for line in lines)
        )
        step_index, step_lines = self.step_lines(workflow, step_name)

        self.assertFalse(
            any(line.startswith("        if:") for line in step_lines)
        )
        self.assertFalse(
            any(line.startswith("        continue-on-error:") for line in step_lines)
        )
        return step_index, step_lines

    def conditionally_active_step_lines(
        self, workflow: str, step_name: str, condition: str
    ) -> tuple[int, list[str]]:
        lines = workflow.splitlines()
        self.assertFalse(any(line.startswith("    if:") for line in lines))
        self.assertFalse(
            any(line.startswith("    continue-on-error:") for line in lines)
        )
        step_index, step_lines = self.step_lines(workflow, step_name)
        self.assertEqual(
            [line for line in step_lines if line.startswith("        if:")],
            [f"        if: {condition}"],
        )
        self.assertFalse(
            any(line.startswith("        continue-on-error:") for line in step_lines)
        )
        return step_index, step_lines

    def step_lines(self, workflow: str, step_name: str) -> tuple[int, list[str]]:
        lines = workflow.splitlines()
        marker = f"      - name: {step_name}"
        self.assertIn(marker, lines)
        step_start = lines.index(marker)
        step_end = next(
            (
                index
                for index in range(step_start + 1, len(lines))
                if lines[index].startswith("      - ")
            ),
            len(lines),
        )
        return workflow.index(marker), lines[step_start:step_end]

    def active_folded_step_tokens(self, workflow: str, step_name: str) -> list[str]:
        _, step_lines = self.active_step_lines(workflow, step_name)
        return self.folded_step_line_tokens(step_lines)

    def folded_step_line_tokens(self, step_lines: list[str]) -> list[str]:
        run_index = step_lines.index("        run: >-")
        command = " ".join(
            line.strip()
            for line in step_lines[run_index + 1 :]
            if line.startswith("          ")
            and line.strip()
            and not line.lstrip().startswith("#")
        )
        return shlex.split(command)

    def test_node_toolchain_contract_rejects_inert_commands(self) -> None:
        active = """      - name: Checkout
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - name: Install pinned Node
        uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
        with:
          node-version-file: .node-version

      - name: Check Node version
        shell: pwsh
        run: |
          $expectedNode = (Get-Content -Raw .node-version).Trim()
          if ((node --version) -ne "v$expectedNode") {
            throw "Node $expectedNode must be active for CI gates"
          }

      - name: Enable Corepack
        shell: pwsh
        run: |
          corepack enable
          $packageManager = (Get-Content -Raw package.json | ConvertFrom-Json).packageManager
          $expectedPnpm = $packageManager -replace '^pnpm@', ''
          if ((pnpm --version) -ne $expectedPnpm) {
            throw "pnpm $expectedPnpm must be active for CI gates"
          }
"""
        self.assert_active_pinned_node_steps(active)

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

    def test_ci_action_pin_contract_rejects_tampering(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        replacements = [
            (CHECKOUT_SHA, "0" * 40),
            (INSTALL_ACTION_SHA, "1" * 40),
            (UPLOAD_ARTIFACT_SHA, "2" * 40),
            ("cargo-nextest@0.9.143", "cargo-nextest@0.9.142"),
        ]

        for expected, replacement in replacements:
            with self.subTest(expected=expected), self.assertRaises(AssertionError):
                tampered = workflow.replace(expected, replacement)
                self.assert_active_pinned_node_steps(tampered)
                self.assert_active_pinned_gate_actions(tampered)

    def test_ci_action_pin_contract_rejects_new_unpinned_actions(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assert_all_remote_actions_pinned(
            workflow
            + "\n      - uses: example/action@"
            + "a" * 40
            + "\n      - uses: ./local-action\n"
        )

        for reference in (
            "example/action@main",
            "example/action@v1",
            "example/action@abc1234",
        ):
            with self.subTest(reference=reference), self.assertRaises(AssertionError):
                self.assert_all_remote_actions_pinned(
                    workflow + f"\n      - uses: {reference}\n"
                )

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


if __name__ == "__main__":
    unittest.main()
