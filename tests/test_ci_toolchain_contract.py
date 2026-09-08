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
    "taiki-e/install-action": "7b8d4719ee4aaa279bdf55df38dacb9ebfe12a6c",
}
TOPIC_BRANCH_TYPES = ("feat", "fix", "docs", "test", "ci", "chore", "refactor")
UBUNTU = "matrix.os == 'ubuntu-24.04'"
NATIVE = "matrix.os != 'windows-latest'"
WINDOWS = "matrix.os == 'windows-latest'"
M12_EXECUTOR = "target/m12-standard/release/flow-executor"
EVIDENCE_ONLY_UNIX_RUNNER_PATTERN = (
    r"flow-agent[\\/]flow-agent-core[\\/]src[\\/]runtime[\\/]"
    r"tool_runner[\\/]unix_process(\.rs|[\\/])"
)
M12_INSTALLER_ACCEPTANCE = ROOT / "scripts" / "run-m12-installer-acceptance.sh"
M12_READINESS_NEGATIVES = ROOT / "scripts" / "run-m12-readiness-negatives.sh"
M12_NATIVE_SUPPORT = ROOT / "flow-agent/flow-agent-executor/tests/native_support/mod.rs"
M12_NATIVE_HELPER = ROOT / "scripts/m12_native.py"


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
    command = step_run(workflow, name).replace("${{ matrix.packages }}", "--workspace")
    return shlex.split(" ".join(command.splitlines()))


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
    product = "steps.scope.outputs.product == 'true'"
    case.assertEqual(conditions, [product if condition is None else f"{product} && ({condition})"])
    case.assertEqual(
        [line for line in lines if line.startswith("        continue-on-error:")],
        ["        continue-on-error: true"] if continue_on_error else [],
    )
    return lines


class CiWorkflowContractTest(unittest.TestCase):
    def test_native_platform_gates_select_the_owned_packages(self) -> None:
        workflow = workflow_text()
        scopes = dict(re.findall(
            r"^          - os: ([^\n]+)\n            packages: ([^\n]+)$",
            workflow,
            re.MULTILINE,
        ))
        self.assertEqual(scopes, {
            "ubuntu-24.04": "--workspace",
            "macos-26": "--workspace",
            "windows-latest": "-p core-script -p core-policy -p proto",
        })
        for name in ("Check lints", "Run tests", "Run Rustdoc tests", "Check shared Windows line coverage"):
            command = step_run(workflow, name)
            self.assertIn("${{ matrix.packages }}", command)
            for os_name, packages in scopes.items():
                with self.subTest(gate=name, platform=os_name):
                    rendered = command.replace("${{ matrix.packages }}", packages)
                    tokens = shlex.split(" ".join(rendered.splitlines()))
                    if os_name == "windows-latest":
                        self.assertEqual(
                            [tokens[index + 1] for index, token in enumerate(tokens) if token == "-p"],
                            ["core-script", "core-policy", "proto"],
                        )
                        self.assertNotIn("--workspace", tokens)
                    else:
                        self.assertIn("--workspace", tokens)
        assert_step_state(
            self, workflow, "Run native release Executor acceptance", condition=NATIVE
        )

    def test_versions_come_from_their_canonical_project_files(self) -> None:
        workflow = workflow_text()
        with (ROOT / "rust-toolchain.toml").open("rb") as toolchain_file:
            rust_version = tomllib.load(toolchain_file)["toolchain"]["channel"]

        self.assertEqual(
            (ROOT / ".node-version").read_text(encoding="utf-8"),
            f"{NODE_VERSION}\n",
        )
        self.assertEqual(PACKAGE["engines"]["node"], ">=24.2.0")
        self.assertGreaterEqual(
            tuple(map(int, NODE_VERSION.split("."))),
            tuple(map(int, PACKAGE["engines"]["node"][2:].split("."))),
        )
        self.assertRegex(PACKAGE["packageManager"], r"^pnpm@\d+\.\d+\.\d+$")
        self.assertIn(
            "node-version-file: .node-version",
            "\n".join(step_lines(workflow, "Install pinned Node")),
        )
        self.assertIn(
            "persist-credentials: false",
            "\n".join(step_lines(workflow, "Checkout")),
        )
        self.assertIn(".node-version", step_run(workflow, "Check Node version"))
        self.assertIn("package.json", step_run(workflow, "Enable Corepack"))
        self.assertIn("rust-toolchain.toml", step_run(workflow, "Select pinned Rust"))
        self.assertNotIn(rust_version, workflow)
        self.assertNotIn("check-latest:", workflow)

    def test_minimum_node_exercises_tooling_after_the_pinned_gates(self) -> None:
        workflow = workflow_text()
        names = ("Select minimum Node", "Install minimum Node", "Check minimum Node tooling")
        for name in names:
            assert_step_state(self, workflow, name, condition=UBUNTU)
        selection = step_run(workflow, names[0])
        self.assertIn("package.json", selection)
        self.assertIn(".engines.node", selection)
        self.assertIn("$env:GITHUB_OUTPUT", selection)
        self.assertIn(
            "node-version: ${{ steps.minimum-node.outputs.version }}",
            "\n".join(step_lines(workflow, names[1])),
        )
        proof = step_run(workflow, names[2])
        self.assertIn('node scripts/run-python.mjs -m unittest discover -s tests -p "test_*.py"', proof)
        self.assertIn("node scripts/check-html-render.mjs", proof)
        self.assertLess(workflow.index("name: Check documentation links"), workflow.index(f"name: {names[0]}"))
        for earlier, later in zip(names, names[1:]):
            self.assertLess(workflow.index(f"name: {earlier}"), workflow.index(f"name: {later}"))

    def test_corepack_is_explicitly_provisioned_before_use(self) -> None:
        workflow = workflow_text()
        assert_step_state(self, workflow, "Install pinned Corepack")
        provision = step_run(workflow, "Install pinned Corepack")
        self.assertIn("Join-Path $env:RUNNER_TEMP watershed-node-tools", provision)
        self.assertRegex(
            provision,
            r"npm install --global --prefix \$toolsRoot corepack@\d+\.\d+\.\d+",
        )
        self.assertIn("$IsWindows", provision)
        self.assertIn("Join-Path $toolsRoot bin", provision)
        self.assertIn("$env:GITHUB_PATH", provision)
        self.assertLess(
            workflow.index("      - name: Install pinned Corepack"),
            workflow.index("      - name: Enable Corepack"),
        )

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
            workflow.replace("report --fail-under-lines 90", "report --fail-under-lines 89", 1),
            workflow.replace("cargo audit", "true", 1),
            workflow.replace("pnpm run docs:render-check", "true", 1),
            workflow.replace("--example m11_budgets", "--example m12_executor_startup", 1),
            workflow.replace("--example m12_executor_startup", "--example m11_budgets", 1),
            workflow.replace(f'-- --executor "$PWD/{M12_EXECUTOR}"', "--", 1),
            workflow.replace("--test native_contract", "--test absent", 1),
            workflow.replace("--test native_self_protection", "--test absent", 1),
            workflow.replace("cargo llvm-cov show-env --sh", "true", 1),
            workflow.replace("cargo llvm-cov report", "true", 1),
            workflow.replace('export M12_COVERAGE_BIN_DIR="$CARGO_TARGET_DIR/debug"',
                             'export M12_COVERAGE_BIN_DIR="target/m12-standard/release"', 1),
            workflow.replace('/bin/sh scripts/run-m12-readiness-negatives.sh', "true", 1),
            workflow.replace("install.tests.test_install install.tests.test_readiness",
                             "install.tests.test_install", 1),
            workflow.replace(
                '\n'.join(step_lines(workflow, "Run M1.2 installer contract tests")),
                '\n'.join(step_lines(workflow, "Run M1.2 installer contract tests"))
                .replace(NATIVE, UBUNTU), 1),
        )
        for index, mutated in enumerate(mutations):
            self.assertTrue(mutated != workflow, "mutation must exercise an existing gate")
            with self.subTest(mutation=index), self.assertRaises(AssertionError):
                self.assert_ci_gate_contract(mutated)

    def test_testing_contract_covers_tooling_and_rustdoc_gates(self) -> None:
        testing = (ROOT / "TESTING.md").read_text(encoding="utf-8")
        for contract in (
            "documentation gates (HTML rendering and link-manifest generation)",
            "the Node advisory audit",
            "the Rust test-isolation runner",
            "`pnpm audit`",
            "`cargo --config .cargo/test-isolation.toml test --locked --workspace --all-features --doc`",
        ):
            self.assertIn(contract, testing)

    def test_native_contract_uses_debug_or_explicit_release_executor(self) -> None:
        self.assertIn(
            'env!("CARGO_BIN_EXE_flow-executor")',
            M12_NATIVE_SUPPORT.read_text(encoding="utf-8"),
        )

    def assert_ci_gate_contract(self, workflow: str) -> None:
        self.assertFalse(any(line.startswith("    if:") for line in workflow.splitlines()))
        self.assertFalse(
            any(line.startswith("    continue-on-error:") for line in workflow.splitlines())
        )
        commands = {
            "Check formatting": "cargo fmt --all --check",
            "Check lints": "cargo clippy --locked ${{ matrix.packages }} --all-targets --all-features -- -D warnings",
            "Check RustSec advisories": "cargo audit",
            "Check dependency policy": "cargo deny check",
            "Check Node advisories": "pnpm audit",
            "Render HTML docs": "pnpm run docs:render-check",
        }
        for name, command in commands.items():
            assert_step_state(self, workflow, name)
            self.assertEqual(step_run(workflow, name), command)

        assert_step_state(self, workflow, "Run M1.2 installer contract tests", condition=NATIVE)
        self.assertEqual(
            folded_tokens(workflow, "Run M1.2 installer contract tests"),
            ["node", "scripts/run-python.mjs", "-m", "unittest",
             "install.tests.test_install", "install.tests.test_readiness"],
        )

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
        assert_step_state(self, workflow, "Check shared Windows line coverage", condition=WINDOWS)
        coverage = folded_tokens(workflow, "Check shared Windows line coverage")
        for required in ("cargo", "llvm-cov", "nextest", "--locked", "--workspace",
                         "--all-targets", "--all-features", "--show-missing-lines"):
            self.assertIn(required, coverage)
        self.assertEqual(coverage[coverage.index("--fail-under-lines") + 1], "90")
        self.assertEqual(coverage[coverage.index("--config") + 1], TEST_ISOLATION)
        exclusions = coverage[coverage.index("--ignore-filename-regex") + 1]
        self.assertIn(EVIDENCE_ONLY_UNIX_RUNNER_PATTERN, exclusions)

        native_lines = assert_step_state(
            self, workflow, "Check native coverage and installation acceptance", condition=NATIVE)
        self.assertIn("        shell: bash", native_lines)
        native = step_run(workflow, "Check native coverage and installation acceptance")
        ordered = (
            "cargo llvm-cov clean --workspace",
            'eval "$(cargo llvm-cov show-env --sh)"',
            "cargo build --locked -p flow-agent-cli -p flow-agent-executor",
            'export M12_COVERAGE_BIN_DIR="$CARGO_TARGET_DIR/debug"',
            "/bin/sh scripts/run-m12-installer-acceptance.sh",
            "cargo nextest run",
            "/bin/sh scripts/run-m12-readiness-negatives.sh",
            "cargo llvm-cov report --fail-under-lines 90",
        )
        for command in ordered:
            self.assertIn(command, native)
        positions = [native.index(command) for command in ordered]
        self.assertEqual(positions, sorted(positions))
        self.assertEqual(native.count("cargo nextest run"), 1)
        native_test = native[native.index("cargo nextest run"):
                             native.index("/bin/sh scripts/run-m12-readiness-negatives.sh")]
        self.assertEqual(shlex.split(native_test.replace("\\\n", " ")), [
            "cargo", "nextest", "run", "--config", TEST_ISOLATION, "--locked",
            "--workspace", "--all-targets", "--all-features",
        ])
        report = shlex.split(native[native.index("cargo llvm-cov report"):].replace("\\\n", " "))
        self.assertEqual(report, ["cargo", "llvm-cov", "report", "--fail-under-lines", "90",
                                  "--ignore-filename-regex", exclusions, "--show-missing-lines"])
        for forbidden in ("--release", "--target ", "--coverage-target-only", "docker ",
                          "runuser ", "systemctl ", "--skip", "|| true"):
            self.assertNotIn(forbidden, native)

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
        host_condition = NATIVE if milestone == "M1.2" else UBUNTU
        condition = f"{host_condition} && !cancelled()" if milestone == "M1.2" else host_condition
        run_lines = assert_step_state(
            self, workflow, run_name, condition=condition, continue_on_error=True
        )
        self.assertIn(f"        id: {run_id}", run_lines)
        run = step_run(workflow, run_name)
        self.assertIn("cargo run --locked -p flow-agent-core --release", run)
        self.assertIn(f"--features {feature} --example {example}", run)
        if milestone == "M1.2":
            self.assertIn(f'-- --executor "$PWD/{M12_EXECUTOR}"', run)
            artifact += "-${{ matrix.os }}"
        self.assertIn(f"> {output}", run)

        upload_name = f"Upload {milestone} " + (
            "performance evidence" if milestone == "M1.1" else "executor startup evidence"
        )
        upload = assert_step_state(
            self, workflow, upload_name, condition=f"{host_condition} && always()"
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
            condition=f"{host_condition} && always() && steps.{run_id}.outcome != 'success'",
        )
        self.assertEqual(step_run(workflow, enforce_name), "exit 1")

    def assert_m12_release_boundary(self, workflow: str) -> None:
        assert_step_state(self, workflow, "Build native installation artifacts", condition=NATIVE)
        build = step_run(workflow, "Build native installation artifacts")
        self.assertIn("cargo build --locked --release -p flow-agent-cli -p flow-agent-executor "
                      "--target-dir target/m12-standard", build)
        self.assertIn("cargo build --locked --release -p flow-agent-cli "
                      "--features m12-install-acceptance --target-dir target/m12-acceptance", build)
        self.assertIn(f"{M12_EXECUTOR} --probe", build)
        self.assertIn('assert probe["ready"] is True', build)
        assert_step_state(self, workflow, "Run native release Executor acceptance", condition=NATIVE)
        release = step_run(workflow, "Run native release Executor acceptance")
        self.assertIn(f'FLOW_EXECUTOR_UNDER_TEST="$PWD/{M12_EXECUTOR}"', release)
        self.assertIn("cargo nextest run --locked -p flow-agent-executor "
                      "--test native_contract --test native_self_protection", release)
        self.assertNotIn("--skip", release)
        self.assertLess(workflow.index("      - name: Build native installation artifacts"),
                        workflow.index("      - name: Run native release Executor acceptance"))
        self.assertLess(workflow.index("      - name: Run native release Executor acceptance"),
                        workflow.index("      - name: Check native coverage and installation acceptance"))
        assert_step_state(self, workflow, "Run public Custom Executor conformance", condition=NATIVE)
        conformance = step_run(workflow, "Run public Custom Executor conformance")
        self.assertIn("--example custom_executor_conformance", conformance)
        self.assertIn(f'--executor "$PWD/{M12_EXECUTOR}"', conformance)

        # CI provisioning is explicit; the product installer never performs it.
        assert_step_state(self, workflow, "Provision native Linux protection prerequisites",
                          condition=UBUNTU)
        prerequisites = step_run(workflow, "Provision native Linux protection prerequisites")
        for required in ("--no-install-recommends bubblewrap apparmor",
                         "/usr/bin/bwrap flags=(unconfined)", "userns,",
                         "apparmor_parser --replace /etc/apparmor.d/watershed-bwrap-userns",
                         'test "$(sysctl -n kernel.apparmor_restrict_unprivileged_userns)" = 1'):
            self.assertIn(required, prerequisites)
        for forbidden in ("sysctl -w", "sysctl --write", "systemctl ", "useradd ", "docker ",
                          "official_linux", "linux_support", "static-self-reexec"):
            self.assertNotIn(forbidden, workflow)

        installer = M12_INSTALLER_ACCEPTANCE.read_text(encoding="utf-8")
        for required in ('target/m12-standard/release/flow "$bundle/flow"',
                         f'{M12_EXECUTOR} "$bundle/flow-executor"',
                         'target/m12-acceptance/release/flow "$acceptance_bundle/flow"',
                         f'{M12_EXECUTOR} "$acceptance_bundle/flow-executor"',
                         'install -m 0755 "$coverage_flow" "$coverage_bundle/flow"',
                         'install -m 0755 "$coverage_executor" "$coverage_bundle/flow-executor"',
                         '--prefix "$standard_prefix"',
                         '--prefix "$custom_prefix" --no-default-executor',
                         'executor configure --default',
                         'assert receipt["self_protection_active"] is True'):
            self.assertIn(required, installer)
        coverage = installer.index('if [ -n "${M12_COVERAGE_BIN_DIR:-}" ]')
        self.assertIn('check_custom_selection\n', installer[:coverage])
        self.assertIn('check_custom_selection\n', installer[coverage:])
        self.assertIn('/bin/sh "$bundle/install.sh" --prefix "$standard_prefix"', installer[coverage:])
        self.assertIn('config="$home/Library/Application Support"', installer)
        self.assertIn('expected_platform=macos-26-aarch64', installer)
        for source in (installer, M12_READINESS_NEGATIVES.read_text(encoding="utf-8")):
            for forbidden in ("systemctl ", "runuser ", "useradd ", "chown ", "/root/", "/work/",
                              "sysctl ", "apparmor_parser", "static-self-reexec"):
                self.assertNotIn(forbidden, source)

    def test_m12_installer_acceptance_prepares_fixture_home_before_init(self):
        installer_acceptance = M12_INSTALLER_ACCEPTANCE.read_text(encoding="utf-8")
        fixture_home_setup = (
            'install -d -m 0700 "$config" "$home" "$agent_home" '
            '"$fixture_home" "$fixture_workspace"'
        )
        fixture_init = (
            'run_in_workspace "$fixture_workspace" /usr/bin/env '
            'FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" init'
        )

        setup = installer_acceptance.index(fixture_home_setup)
        initialization = installer_acceptance.index(fixture_init)
        self.assertLess(setup, initialization)
        self.assertIn('mktemp -d "$RUNNER_TEMP/m12-installer.XXXXXX"', installer_acceptance)

    def test_m12_installer_acceptance_has_finite_liveness_bounds(self) -> None:
        installer = M12_INSTALLER_ACCEPTANCE.read_text(encoding="utf-8")
        readiness = M12_READINESS_NEGATIVES.read_text(encoding="utf-8")
        helper = M12_NATIVE_HELPER.read_text(encoding="utf-8")
        self.assertIn('exec node scripts/run-python.mjs scripts/m12_native.py acceptance "$0"', installer)
        self.assertIn("exec node scripts/run-python.mjs scripts/m12_native.py readiness", readiness)
        self.assertIn('run(["/bin/sh", sys.argv[2], "--bounded"], timeout=600, capture=False)', helper)
        for required in ("timeout=20", "start_new_session=True",
                         "child.communicate(timeout=timeout)",
                         "os.killpg(child.pid, signal.SIGTERM)",
                         "os.killpg(child.pid, signal.SIGKILL)",
                         "child.communicate(timeout=2)",
                         'assert checked.returncode == 65',
                         'assert checked.stdout == b""',
                         'assert installed.returncode == 1',
                         'assert list((prefix / "bin").iterdir()) == []'):
            self.assertIn(required, helper)
        self.assertNotIn("subprocess.STDOUT", helper)


if __name__ == "__main__":
    unittest.main()
