import json
import re
import shlex
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_PATH = ROOT / ".github" / "workflows" / "ci.yml"
PACKAGE = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
NODE_VERSION = (ROOT / ".node-version").read_text(encoding="utf-8").strip()
TEST_ISOLATION = (
    'target."cfg(all())".runner = ["node", "../../scripts/run-isolated-rust-test.mjs"]'
)
ACTION_PINS = {
    "actions/checkout": "3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/setup-node": "820762786026740c76f36085b0efc47a31fe5020",
    "actions/upload-artifact": "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    "taiki-e/install-action": "1ed6d7be6168f6c9046541087ff549b6bc581fdf",
}
TOPIC_BRANCH_TYPES = ("feat", "fix", "docs", "test", "ci", "chore", "refactor")
UBUNTU = "matrix.os == 'ubuntu-24.04'"
NON_UBUNTU = "matrix.os != 'ubuntu-24.04'"
M12_TARGET = "x86_64-unknown-linux-musl"
M12_EXECUTOR = f"target/{M12_TARGET}/release/flow-executor"


def workflow_text() -> str:
    return WORKFLOW_PATH.read_text(encoding="utf-8")


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
    for line in lines[branches_start + 1 : push_end]:
        item = line.strip()
        if item.startswith("- ") and not item.startswith("- #"):
            branches.append(item[2:].strip().strip('"'))
    return tuple(branches)


def step_lines(workflow: str, name: str) -> list[str]:
    lines = workflow.splitlines()
    marker = f"      - name: {name}"
    if marker not in lines:
        raise AssertionError(f"missing CI step: {name}")
    start = lines.index(marker)
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index].startswith("      - ")
        ),
        len(lines),
    )
    return lines[start:end]


def step_run(workflow: str, name: str) -> str:
    lines = step_lines(workflow, name)
    run_index = next(
        (index for index, line in enumerate(lines) if line.startswith("        run:")),
        None,
    )
    if run_index is None:
        raise AssertionError(f"CI step has no command: {name}")
    declaration = lines[run_index].removeprefix("        run:").strip()
    if declaration not in ("|", ">-"):
        return declaration
    return "\n".join(
        line.removeprefix("          ")
        for line in lines[run_index + 1 :]
        if line.startswith("          ")
    ).rstrip()


def folded_tokens(workflow: str, name: str) -> list[str]:
    return shlex.split(" ".join(step_run(workflow, name).splitlines()))


def assert_step_state(
    case: unittest.TestCase,
    workflow: str,
    name: str,
    *,
    condition: str | None = None,
    continue_on_error: bool = False,
) -> list[str]:
    lines = step_lines(workflow, name)
    conditions = [
        line.removeprefix("        if: ")
        for line in lines
        if line.startswith("        if:")
    ]
    case.assertEqual(conditions, [] if condition is None else [condition])
    case.assertEqual(
        [line for line in lines if line.startswith("        continue-on-error:")],
        ["        continue-on-error: true"] if continue_on_error else [],
    )
    return lines


class CiWorkflowContractTest(unittest.TestCase):
    def test_versions_come_from_their_canonical_project_files(self) -> None:
        workflow = workflow_text()
        with (ROOT / "rust-toolchain.toml").open("rb") as toolchain_file:
            rust_version = tomllib.load(toolchain_file)["toolchain"]["channel"]

        self.assertEqual(
            (ROOT / ".node-version").read_text(encoding="utf-8"),
            f"{NODE_VERSION}\n",
        )
        self.assertRegex(PACKAGE["packageManager"], r"^pnpm@\d+\.\d+\.\d+$")
        self.assertIn(
            "node-version-file: .node-version",
            "\n".join(step_lines(workflow, "Install pinned Node")),
        )
        self.assertIn(".node-version", step_run(workflow, "Check Node version"))
        self.assertIn("package.json", step_run(workflow, "Enable Corepack"))
        self.assertIn("rust-toolchain.toml", step_run(workflow, "Select pinned Rust"))
        self.assertNotIn(rust_version, workflow)
        self.assertNotIn("check-latest:", workflow)

    def test_remote_actions_are_reviewed_and_immutable(self) -> None:
        workflow = workflow_text()
        seen = set()
        for line in workflow.splitlines():
            match = re.match(r"^\s*(?:-\s+)?uses:\s*([^\s#]+)", line)
            if match is None or match.group(1).startswith("./"):
                continue
            reference = match.group(1)
            self.assertRegex(reference, r"^[^@/\s]+/[^@\s]+@[0-9a-f]{40}$")
            action, sha = reference.rsplit("@", 1)
            self.assertEqual(sha, ACTION_PINS[action])
            seen.add(action)
        self.assertEqual(seen, set(ACTION_PINS))

    def test_ci_runs_on_every_permitted_topic_branch(self) -> None:
        self.assertEqual(
            ci_push_branches(workflow_text()),
            ("main", *(f"{kind}/**" for kind in TOPIC_BRANCH_TYPES)),
        )

    def test_ci_branch_parser_ignores_comments_and_other_trigger_keys(self) -> None:
        workflow = """on:
  push:
    paths:
      - "feat/**"
    branches:
      # "fix/**"
      - main
"""
        self.assertEqual(ci_push_branches(workflow), ("main",))

    def test_feature_gated_evidence_reporters_are_registered(self) -> None:
        manifest = tomllib.loads(
            (ROOT / "flow-agent" / "flow-agent-core" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        examples = {example["name"]: example for example in manifest["example"]}
        self.assertEqual(
            examples["m11_budgets"]["required-features"], ["m11-budget-evidence"]
        )
        self.assertEqual(
            examples["m12_executor_startup"]["required-features"],
            ["m12-startup-evidence"],
        )
        self.assert_ci_gate_contract(workflow_text())

    def test_mandatory_gate_mutations_are_rejected(self) -> None:
        workflow = workflow_text()
        mutations = (
            workflow.replace("cargo fmt --all --check", "true", 1),
            workflow.replace("--fail-under-lines 90", "--fail-under-lines 89", 1),
            workflow.replace("cargo audit", "true", 1),
            workflow.replace("pnpm run docs:render-check", "true", 1),
            workflow.replace("--example m11_budgets", "--example m12_executor_startup", 1),
            workflow.replace("--example m12_executor_startup", "--example m11_budgets", 1),
            workflow.replace(
                '-- --executor "$PWD/target/x86_64-unknown-linux-musl/release/flow-executor"',
                "--",
                1,
            ),
            workflow.replace("cargo test --locked -p flow-agent-executor", "true", 1),
            workflow.replace("grep -q 'INTERP'", "grep -q 'NOT_INTERP'", 1),
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated), self.assertRaises(AssertionError):
                self.assert_ci_gate_contract(mutated)

    def test_m12_release_boundary_is_real_and_fail_closed(self) -> None:
        self.assert_m12_release_boundary(workflow_text())

    def test_testing_contract_covers_tooling_and_rustdoc_gates(self) -> None:
        testing = (ROOT / "TESTING.md").read_text(encoding="utf-8")
        for contract in (
            "documentation gates (HTML rendering and link-manifest generation)",
            "the Node advisory audit",
            "the Rust test-isolation runner",
            "`pnpm audit --lockfile-only`",
            "`cargo --config .cargo/test-isolation.toml test --locked --workspace --all-features --doc`",
        ):
            self.assertIn(contract, testing)

    def assert_ci_gate_contract(self, workflow: str) -> None:
        self.assertFalse(any(line.startswith("    if:") for line in workflow.splitlines()))
        self.assertFalse(
            any(line.startswith("    continue-on-error:") for line in workflow.splitlines())
        )
        commands = {
            "Check formatting": "cargo fmt --all --check",
            "Check lints": "cargo clippy --locked --workspace --all-targets --all-features -- -D warnings",
            "Check RustSec advisories": "cargo audit",
            "Check dependency policy": "cargo deny check",
            "Check Node advisories": "pnpm audit --lockfile-only",
            "Render HTML docs": "pnpm run docs:render-check",
        }
        for name, command in commands.items():
            assert_step_state(self, workflow, name)
            self.assertEqual(step_run(workflow, name), command)

        self.assertEqual(
            folded_tokens(workflow, "Run tests"),
            [
                "cargo",
                "nextest",
                "run",
                "--config",
                TEST_ISOLATION,
                "--locked",
                "--workspace",
                "--all-targets",
                "--all-features",
            ],
        )
        self.assertEqual(
            folded_tokens(workflow, "Run Rustdoc tests"),
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
        coverage = folded_tokens(workflow, "Check line coverage")
        for required in (
            "cargo",
            "llvm-cov",
            "nextest",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--show-missing-lines",
        ):
            self.assertIn(required, coverage)
        self.assertEqual(coverage[coverage.index("--fail-under-lines") + 1], "90")
        self.assertEqual(coverage[coverage.index("--config") + 1], TEST_ISOLATION)

        self.assert_evidence_gate(
            workflow,
            milestone="M1.1",
            run_name="Run M1.1 performance evidence",
            run_id="m11_evidence",
            feature="m11-budget-evidence",
            example="m11_budgets",
            artifact="m11-performance-evidence",
            output="target/m11-performance/m11-performance-evidence.jsonl",
        )
        self.assert_evidence_gate(
            workflow,
            milestone="M1.2",
            run_name="Run M1.2 executor startup evidence",
            run_id="m12_startup_evidence",
            feature="m12-startup-evidence",
            example="m12_executor_startup",
            artifact="m12-executor-startup-evidence",
            output="target/m12-startup/m12-executor-startup.jsonl",
        )
        self.assert_m12_release_boundary(workflow)
        link_step = step_run(workflow, "Check documentation links")
        self.assertIn("scripts/list-tracked-files.mjs '*.md' '*.html'", link_step)
        self.assertIn("lychee --no-progress --include-fragments -- @docs", link_step)

    def assert_evidence_gate(
        self,
        workflow: str,
        *,
        milestone: str,
        run_name: str,
        run_id: str,
        feature: str,
        example: str,
        artifact: str,
        output: str,
    ) -> None:
        run_lines = assert_step_state(
            self, workflow, run_name, condition=UBUNTU, continue_on_error=True
        )
        self.assertIn(f"        id: {run_id}", run_lines)
        run = step_run(workflow, run_name)
        self.assertIn("cargo run --locked -p flow-agent-core --release", run)
        self.assertIn(f"--features {feature} --example {example}", run)
        if milestone == "M1.2":
            self.assertIn(f'-- --executor "$PWD/{M12_EXECUTOR}"', run)
        self.assertIn(f"> {output}", run)

        upload_name = f"Upload {milestone} " + (
            "performance evidence" if milestone == "M1.1" else "executor startup evidence"
        )
        upload = assert_step_state(
            self, workflow, upload_name, condition=f"{UBUNTU} && always()"
        )
        joined = "\n".join(upload)
        self.assertIn(f"name: {artifact}", joined)
        self.assertIn(f"path: {output}", joined)
        self.assertIn("if-no-files-found: error", joined)

        enforce_name = f"Enforce {milestone} " + (
            "evidence integrity" if milestone == "M1.1" else "executor startup evidence"
        )
        assert_step_state(
            self,
            workflow,
            enforce_name,
            condition=f"{UBUNTU} && steps.{run_id}.outcome != 'success'",
        )
        self.assertEqual(step_run(workflow, enforce_name), "exit 1")

    def assert_m12_release_boundary(self, workflow: str) -> None:
        single_commands = {
            "Install M1.2 executor target": f"rustup target add {M12_TARGET}",
            "Build M1.2 executor": (
                "cargo build --locked --release -p flow-agent-executor "
                f"--bin flow-executor --target {M12_TARGET}"
            ),
            "Build M1.2 dynamic rejection fixture": (
                "cargo build --locked --release -p flow-agent-executor --bin flow-executor"
            ),
            "Build M1.2 installer CLI": (
                "cargo build --locked --release -p flow-agent-cli --bin flow"
            ),
            "Run M1.2 installer contract tests": (
                "node scripts/run-python.mjs -m unittest install.tests.test_install"
            ),
        }
        for name, command in single_commands.items():
            assert_step_state(self, workflow, name, condition=UBUNTU)
            self.assertEqual(step_run(workflow, name), command)

        static = "\n".join(
            assert_step_state(
                self, workflow, "Check M1.2 executor is static", condition=UBUNTU
            )
        )
        for required in (M12_EXECUTOR, "readelf -l", "grep -q 'INTERP'", "exit 1"):
            self.assertIn(required, static)

        bwrap = "\n".join(
            assert_step_state(
                self, workflow, "Check M1.2 Bubblewrap availability", condition=UBUNTU
            )
        )
        for required in ("/usr/bin/bwrap", "dpkg-query", "bwrap --version"):
            self.assertIn(required, bwrap)

        executor_tests = "\n".join(
            assert_step_state(
                self, workflow, "Run M1.2 executor tests", condition=UBUNTU
            )
        )
        self.assertIn("BWRAP_UNDER_TEST: /usr/bin/bwrap", executor_tests)
        self.assertIn(
            "FLOW_EXECUTOR_DYNAMIC_UNDER_TEST: ${{ github.workspace }}/target/release/flow-executor",
            executor_tests,
        )
        self.assertIn(f"/{M12_EXECUTOR}", executor_tests)
        self.assertEqual(
            step_run(workflow, "Run M1.2 executor tests"),
            "cargo test --locked -p flow-agent-executor",
        )

        assert_step_state(
            self, workflow, "Run M1.2 installer acceptance", condition=UBUNTU
        )
        installer = step_run(workflow, "Run M1.2 installer acceptance")
        for required in (
            "install/install.sh",
            "target/release/flow",
            M12_EXECUTOR,
            'HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$standard_prefix"',
            '"$standard_prefix/bin/flow" executor check',
            'HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$custom_prefix" --no-default-executor',
            'test ! -e "$custom_prefix/bin/flow-executor"',
            'test "$unavailable_status" -eq 65',
            "executor_unavailable:*",
            'executor configure --path "$executor"',
            '"$custom_prefix/bin/flow" executor check',
        ):
            self.assertIn(required, installer)

        assert_step_state(
            self, workflow, "Check M1.2 unsupported platforms", condition=NON_UBUNTU
        )
        self.assertEqual(
            step_run(workflow, "Check M1.2 unsupported platforms"),
            "cargo test --locked -p flow-agent-cli "
            "executor_check_fails_closed_on_unsupported_platform",
        )


if __name__ == "__main__":
    unittest.main()
