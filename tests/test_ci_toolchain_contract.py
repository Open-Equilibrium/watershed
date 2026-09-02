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
    "taiki-e/install-action": "e67fa11c4b9316fa714ddf0abed07a0c3143b95b",
}
TOPIC_BRANCH_TYPES = ("feat", "fix", "docs", "test", "ci", "chore", "refactor")
UBUNTU = "matrix.os == 'ubuntu-24.04'"
NON_UBUNTU = "matrix.os != 'ubuntu-24.04'"
M12_TARGET = "x86_64-unknown-linux-musl"
M12_EXECUTOR = f"target/{M12_TARGET}/release/flow-executor"
M12_STANDARD_CLI = "target/m12-standard/release/flow"
M12_ACCEPTANCE_CLI = "target/release/flow"
M12_INSTALLED_EXECUTOR = "/usr/local/libexec/watershed/flow-executor"
M12_CONTAINER = "watershed-m12"
M12_COVERAGE_USER = "watershed"
M12_COVERAGE_ENV = "/root/m12-coverage.env"
M12_INSTALLER_ACCEPTANCE = ROOT / "scripts" / "run-m12-installer-acceptance.sh"


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
        self.assertIn(
            "persist-credentials: false",
            "\n".join(step_lines(workflow, "Checkout")),
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
                f'-- --executor {M12_INSTALLED_EXECUTOR}',
                "--",
                1,
            ),
            workflow.replace("cargo test --locked -p flow-agent-executor", "true", 1),
            workflow.replace("cargo llvm-cov show-env --sh", "true", 1),
            workflow.replace("cargo llvm-cov report", "true", 1),
            workflow.replace('grep -q "INTERP"', 'grep -q "NOT_INTERP"', 1),
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
        assert_step_state(
            self, workflow, "Check line coverage", condition=NON_UBUNTU
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

        coverage_prepare_lines = assert_step_state(
            self,
            workflow,
            "Prepare instrumented M1.2 coverage artifacts",
            condition=UBUNTU,
        )
        coverage_prepare = step_run(
            workflow, "Prepare instrumented M1.2 coverage artifacts"
        )
        for required in (
            "Signature: 8a477f597d28d172789f06886806bc55",
            "target/CACHEDIR.TAG",
            f"cargo llvm-cov show-env --sh --target {M12_TARGET} > {M12_COVERAGE_ENV}",
            f". {M12_COVERAGE_ENV}",
            "cargo llvm-cov clean --workspace",
            f"cargo clean --release --target {M12_TARGET} -p flow-agent-executor",
            "cargo clean --release -p flow-agent-cli "
            "--target-dir target/m12-standard",
            "cargo build --locked --release -p flow-agent-executor "
            f"--bin flow-executor --target {M12_TARGET}",
            "cargo build --locked --release -p flow-agent-executor --bin flow-executor",
            "cargo build --locked --release -p flow-agent-cli --bin flow "
            "--target-dir target/m12-standard",
            "cargo build --locked --release -p flow-agent-cli --bin flow "
            "--features m12-install-acceptance",
        ):
            self.assertIn(required, coverage_prepare)
        self.assertIn("        shell: bash", coverage_prepare_lines)
        prepare_order = (
            "target/CACHEDIR.TAG",
            "cargo llvm-cov show-env",
            f". {M12_COVERAGE_ENV}",
            "cargo llvm-cov clean",
            "cargo clean --release",
            "cargo build",
        )
        positions = [coverage_prepare.index(command) for command in prepare_order]
        self.assertEqual(positions, sorted(positions))

        linux_coverage_lines = assert_step_state(
            self, workflow, "Check Linux line coverage", condition=UBUNTU
        )
        linux_coverage = step_run(workflow, "Check Linux line coverage")
        self.assertIn("docker exec", linux_coverage)
        self.assertIn(M12_CONTAINER, linux_coverage)
        for required in (
            f". {M12_COVERAGE_ENV}",
            'install -d -m 0700 "$RUNNER_TEMP"',
            "FLOW_EXECUTOR_DYNAMIC_UNDER_TEST=/work/target/release/flow-executor",
            f"FLOW_EXECUTOR_UNDER_TEST=/work/{M12_EXECUTOR}",
            "cargo nextest run",
            "cargo llvm-cov report",
            f"runuser --user {M12_COVERAGE_USER} --preserve-environment",
            "^Cap(Inh|Prm|Eff|Amb):[[:space:]]+0{16}$",
            "--fail-under-lines 90",
            "--show-missing-lines",
        ):
            self.assertIn(required, linux_coverage)
        self.assertIn("        shell: bash", linux_coverage_lines)
        self.assertIn(TEST_ISOLATION, linux_coverage)
        report = linux_coverage[linux_coverage.index("cargo llvm-cov report") :]
        self.assertIn("--release", report)
        self.assertIn(f"--target {M12_TARGET}", report)
        ordered = (
            f". {M12_COVERAGE_ENV}",
            "cargo nextest run",
            "cargo llvm-cov report",
        )
        positions = [linux_coverage.index(command) for command in ordered]
        self.assertEqual(positions, sorted(positions))
        self.assertNotIn("/tmp/flow-executor-", linux_coverage)
        self.assertNotIn("scripts/run-m12-installer-acceptance.sh", linux_coverage)
        self.assertNotIn("m12-install-acceptance", linux_coverage)
        privilege_drop = linux_coverage.index(
            f"/usr/sbin/runuser --user {M12_COVERAGE_USER} --preserve-environment"
        )
        nextest = linux_coverage.index("cargo nextest run")
        report = linux_coverage.index("cargo llvm-cov report")
        self.assertLess(privilege_drop, nextest)
        self.assertLess(privilege_drop, report)
        self.assertNotIn("cargo nextest run", linux_coverage[:privilege_drop])
        self.assertIn(
            "chown -R watershed:watershed /cargo /work/target", linux_coverage
        )

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
        condition = f"{UBUNTU} && !cancelled()" if milestone == "M1.2" else UBUNTU
        run_lines = assert_step_state(
            self, workflow, run_name, condition=condition, continue_on_error=True
        )
        self.assertIn(f"        id: {run_id}", run_lines)
        run = step_run(workflow, run_name)
        self.assertIn("cargo run --locked -p flow-agent-core --release", run)
        self.assertIn(f"--features {feature} --example {example}", run)
        if milestone == "M1.2":
            self.assertIn(
                "install -d -m 0755 /usr/local/libexec/watershed",
                run,
            )
            self.assertIn(
                f"install -m 0755 {M12_EXECUTOR} {M12_INSTALLED_EXECUTOR}",
                run,
            )
            self.assertIn(f"-- --executor {M12_INSTALLED_EXECUTOR}", run)
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
        contract_images = re.findall(
            r"^      M12_CONTRACT_IMAGE: (ubuntu:24\.04@sha256:[0-9a-f]{64})$",
            workflow,
            re.MULTILINE,
        )
        self.assertEqual(len(contract_images), 1)
        contract_image = contract_images[0]
        self.assertEqual(workflow.count(contract_image), 1)

        single_commands = {
            "Install M1.2 executor target": f"rustup target add {M12_TARGET}",
            "Run M1.2 installer contract tests": (
                "node scripts/run-python.mjs -m unittest install.tests.test_install"
            ),
        }
        for name, command in single_commands.items():
            assert_step_state(self, workflow, name, condition=UBUNTU)
            self.assertEqual(step_run(workflow, name), command)

        apparmor = "\n".join(
            assert_step_state(
                self,
                workflow,
                "Authorize Bubblewrap user namespaces",
                condition=UBUNTU,
            )
        )
        for required in (
            "--no-install-recommends apparmor",
            "/etc/apparmor.d/watershed-bwrap-userns",
            "/usr/bin/bwrap flags=(unconfined)",
            "userns,",
            "apparmor_parser --replace",
            "kernel.apparmor_restrict_unprivileged_userns",
        ):
            self.assertIn(required, apparmor)
        for forbidden in (
            "apparmor-profiles",
            "/usr/share/apparmor",
            "sysctl --write",
            "sysctl -w",
            "CAP_NET_ADMIN",
        ):
            self.assertNotIn(forbidden, apparmor)
        self.assertLess(
            workflow.index("      - name: Authorize Bubblewrap user namespaces"),
            workflow.index("      - name: Run M1.2 executor tests"),
        )

        start = "\n".join(
            assert_step_state(
                self, workflow, "Start controlled M1.2 Ubuntu", condition=UBUNTU
            )
        )
        for required in (
            "$M12_CONTRACT_IMAGE",
            "docker build",
            "systemd dbus-user-session",
            "--privileged",
            "--cgroupns=private",
            "--security-opt apparmor=unconfined",
            "--tmpfs /run",
            "/sbin/init",
            'src=$GITHUB_WORKSPACE,dst=/work,readonly',
            "dst=/opt/rust,readonly",
            "dst=/opt/cargo-registry,readonly",
            "type=volume,dst=/work/target",
            "dst=/usr/local/bin/cargo-llvm-cov,readonly",
            "dst=/usr/local/bin/cargo-nextest,readonly",
            "dst=/usr/local/bin/node,readonly",
        ):
            self.assertIn(required, start)
        self.assertNotIn(contract_image, start)
        for forbidden in (
            "--init",
            "--cgroupns=host",
            "src=/sys/fs/cgroup",
            "sleep infinity",
            "/var/run/docker.sock",
            "sysctl",
            "apparmor_parser",
            'src=$HOME,dst=',
        ):
            self.assertNotIn(forbidden, start)

        provision = "\n".join(
            assert_step_state(
                self, workflow, "Provision M1.2 Ubuntu dependencies", condition=UBUNTU
            )
        )
        for required in (
            f"docker exec {M12_CONTAINER}",
            "bubblewrap",
            "binutils",
            "build-essential",
            "ca-certificates",
            "musl-tools",
            "passwd",
            "python3",
            "procps",
            "util-linux",
            "/opt/cargo-registry",
            "useradd --create-home --uid 10001 --shell /bin/sh watershed",
            "systemd --version",
            'test "$2" = 255',
            "systemctl start user-runtime-dir@10001.service user@10001.service",
            "systemctl is-active --quiet user@10001.service",
            "test -S /run/user/10001/bus",
        ):
            self.assertIn(required, provision)
        self.assertNotIn("loginctl enable-linger", workflow)
        self.assertNotIn("uname -r", provision)

        for name, command in {
            "Build M1.2 executor": (
                "cargo build --locked --release -p flow-agent-executor "
                f"--bin flow-executor --target {M12_TARGET}"
            ),
            "Build M1.2 dynamic rejection fixture": (
                "cargo build --locked --release -p flow-agent-executor --bin flow-executor"
            ),
        }.items():
            assert_step_state(self, workflow, name, condition=UBUNTU)
            build = step_run(workflow, name)
            self.assertIn("docker exec", build)
            self.assertIn(M12_CONTAINER, build)
            self.assertIn(command, build)
            self.assertIn("CARGO_NET_OFFLINE=true", build)
        static = "\n".join(
            assert_step_state(
                self, workflow, "Check M1.2 executor is static", condition=UBUNTU
            )
        )
        for required in (M12_EXECUTOR, "readelf -l", 'grep -q "INTERP"', "exit 1"):
            self.assertIn(required, static)

        executor_tests = "\n".join(
            assert_step_state(
                self, workflow, "Run M1.2 executor tests", condition=UBUNTU
            )
        )
        self.assertIn("BWRAP_UNDER_TEST=/usr/bin/bwrap", executor_tests)
        self.assertIn(
            "FLOW_EXECUTOR_DYNAMIC_UNDER_TEST=/work/target/release/flow-executor",
            executor_tests,
        )
        self.assertIn(f"FLOW_EXECUTOR_UNDER_TEST=/work/{M12_EXECUTOR}", executor_tests)
        for required in (
            "XDG_RUNTIME_DIR=/run/user/10001",
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/10001/bus",
            f"runuser --user {M12_COVERAGE_USER} --preserve-environment",
        ):
            self.assertIn(required, executor_tests)
        self.assertIn(
            "cargo test --locked -p flow-agent-executor --test official_linux",
            step_run(workflow, "Run M1.2 executor tests"),
        )

        conformance = step_run(workflow, "Run public Custom Executor conformance")
        self.assertIn(
            "install -d -m 0755 /usr/local/libexec/watershed",
            conformance,
        )
        self.assertIn(
            f"install -m 0755 /work/{M12_EXECUTOR} {M12_INSTALLED_EXECUTOR}",
            conformance,
        )
        self.assertIn(f"--executor {M12_INSTALLED_EXECUTOR}", conformance)
        self.assertNotIn(f"--executor /work/{M12_EXECUTOR}", conformance)

        assert_step_state(
            self, workflow, "Run M1.2 installer acceptance", condition=UBUNTU
        )
        installer = step_run(workflow, "Run M1.2 installer acceptance")
        self.assertIn("docker exec", installer)
        self.assertIn(M12_CONTAINER, installer)
        self.assertIn("--env RUNNER_TEMP=/opt/watershed-m12", installer)
        self.assertNotIn("--env RUNNER_TEMP=/tmp/", installer)
        self.assertIn(f". {M12_COVERAGE_ENV}", installer)
        self.assertIn("scripts/run-m12-installer-acceptance.sh", installer)
        installer_contract = M12_INSTALLER_ACCEPTANCE.read_text(encoding="utf-8")
        for required in (
            "install/install.sh",
            M12_STANDARD_CLI,
            M12_ACCEPTANCE_CLI,
            M12_EXECUTOR,
            'HOME="$home" XDG_CONFIG_HOME="$config" SUDO_USER=watershed /bin/sh "$bundle/install.sh" --prefix "$standard_prefix"',
            '/usr/sbin/runuser --user watershed --preserve-environment',
            'XDG_RUNTIME_DIR="$user_runtime"',
            'DBUS_SESSION_BUS_ADDRESS="$user_bus"',
            'run_as_watershed "$standard_prefix/bin/flow" executor check',
            'HOME="$home" XDG_CONFIG_HOME="$config" SUDO_USER=watershed /bin/sh "$acceptance_bundle/install.sh" --prefix "$acceptance_prefix"',
            'FLOW_AGENT_M12_INSTALL_ACCEPTANCE=1',
            '"$acceptance_prefix/bin/flow" run smoke-flow',
            '/usr/bin/python3 - "$agent_home"',
            'home / "workspaces"',
            'workspace-v1-*/sessions/*/runs/*/run-log.jsonl',
            'test ! -e "$config/flow-agent/executor.json"',
            '"attempt_kind") == "provider"',
            '"attempt_kind") == "tool"',
            'durable["schema"] == "flow-tool-attempt-output-v1"',
            'receipt["executor"] == "flow-executor"',
            'HOME="$home" XDG_CONFIG_HOME="$config" /bin/sh "$bundle/install.sh" --prefix "$custom_prefix" --no-default-executor',
            'test ! -e "$custom_prefix/bin/flow-executor"',
            'test "$unavailable_status" -eq 65',
            '"error: executor_unavailable:"*',
            'run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" init --registry-root registry',
            'flow-agent/fixtures/smoke-flow/registry/. "$fixture_home/registry/"',
            'run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" validate smoke-flow',
            'run_in_workspace "$fixture_workspace" /usr/bin/env FLOW_AGENT_HOME="$fixture_home" "$custom_prefix/bin/flow" run smoke-flow --emit jsonl',
            'fixture run failed with exit',
            'diff -u flow-agent/fixtures/smoke-flow/expected/smoke-flow.jsonl "$fixture_output"',
            'run_in_workspace "$unavailable_workspace" /usr/bin/env FLOW_AGENT_HOME="$agent_home" "$custom_prefix/bin/flow" run smoke-flow',
            'productive run without an Executor returned exit',
            'productive run without an Executor returned an unexpected diagnostic',
            'productive Executor preflight mutated the workspace',
            'executor="$bundle/flow-executor"',
            'run_as_watershed "$custom_prefix/bin/flow" executor configure --path "$executor"',
            'run_as_watershed "$custom_prefix/bin/flow" executor check',
            'run_as_watershed "$standard_prefix/bin/flow" executor configure --default',
            'run_as_watershed "$standard_prefix/bin/flow" executor check',
        ):
            self.assertIn(required, installer_contract)
        self.assertNotIn("/conversations/", installer_contract)
        self.assertNotIn('executor="$PWD/target/', installer_contract)

        cleanup = assert_step_state(
            self,
            workflow,
            "Stop controlled M1.2 Ubuntu",
            condition=f"{UBUNTU} && always()",
        )
        self.assertIn(
            f"docker rm --force {M12_CONTAINER}",
            "\n".join(cleanup),
        )

        assert_step_state(
            self, workflow, "Check M1.2 unsupported platforms", condition=NON_UBUNTU
        )
        self.assertEqual(
            step_run(workflow, "Check M1.2 unsupported platforms"),
            "cargo nextest run --locked -p flow-agent-cli --test cli "
            "-E 'test(=executor::executor_commands_fail_closed_on_unsupported_platform_without_config_mutation)'",
        )

        evidence = folded_tokens(workflow, "Run M1.2 executor startup evidence")
        for environment_name in (
            "GITHUB_SHA",
            "ImageOS",
            "ImageVersion",
            "M12_CONTRACT_IMAGE",
        ):
            self.assertIn(environment_name, evidence)


if __name__ == "__main__":
    unittest.main()
