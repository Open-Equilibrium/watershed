import json
import os
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

from test_ci_toolchain_contract import ROOT, job_steps, jobs, step_lines, step_run, workflow_text

PRODUCTS = ("flow_agent", "meta_harness", "liquid")
LIQUID_PACKAGES = "-p meta-harness-core -p liquid_bridge"


def scope(docs: bool, *products: str) -> dict[str, str]:
    selected = set(PRODUCTS if products == ("all",) else products)
    outputs = {"docs": docs, "product": bool(selected)}
    outputs.update((product, product in selected) for product in PRODUCTS)
    return {
        **{key: str(value).lower() for key, value in outputs.items()},
        "rust_packages": "--workspace" if "flow_agent" in selected else LIQUID_PACKAGES,
    }


SETUP = scope(False)
DOCS = scope(True)
ALL = scope(True, "all")


class CiProductScopeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.name", "Scope fixture")
        self.git("config", "user.email", "scope@example.invalid")
        self.git("config", "commit.gpgSign", "false")
        self.git("config", "core.autocrlf", "false")
        self.base = self.commit({
            "README.md": "Product documentation\n",
            "src/main.rs": "fn main() {}\n",
        })
        self.git("update-ref", "refs/remotes/origin/main", self.base)

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout.strip()

    def commit(self, files: dict[str, str]) -> str:
        for name, content in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Scope fixture")
        return self.git("rev-parse", "HEAD")

    def run_scope(self, event_name: str, event: dict) -> tuple[subprocess.CompletedProcess, str]:
        event_path = self.root / "event.json"
        output_path = self.root / "output"
        event_path.write_text(json.dumps(event), encoding="utf-8")
        output_path.write_text("", encoding="utf-8")
        result = subprocess.run(
            ["node", str(ROOT / "scripts" / "ci-product-scope.mjs")],
            cwd=self.repo,
            env={
                **os.environ,
                "GITHUB_EVENT_NAME": event_name,
                "GITHUB_EVENT_PATH": str(event_path),
                "GITHUB_OUTPUT": str(output_path),
            },
            capture_output=True,
            text=True,
        )
        return result, output_path.read_text(encoding="utf-8")

    def classify(self, event_name: str, event: dict, expected: dict[str, str] | None) -> dict:
        result, output = self.run_scope(event_name, event)
        if expected is None:
            self.assertEqual(result.returncode, 1)
            self.assertIn("Cannot compare CI product changes", result.stderr)
            self.assertEqual(output, "")
            return {}
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = output.splitlines()
        outputs = dict(line.split("=", 1) for line in lines)
        self.assertEqual(len(lines), len(outputs))
        if expected:
            self.assertEqual(outputs, expected)
        return outputs

    def push(self, before: str, expected: dict[str, str]) -> None:
        self.classify("push", {"before": before}, expected)

    def test_setup_only_changes_do_not_hide_product_changes_in_a_pull_request(self) -> None:
        setup = self.commit({
            "AGENTS.md": "Fixture instructions\n",
            ".agents/skills/fixture/SKILL.md": "Fixture skill\n",
            ".codex/agents/fixture.toml": "fixture = true\n",
        })
        for event_name, event in (
            ("push", {"before": self.base}),
            ("push", {"before": "0" * 40}),
            ("pull_request", {"pull_request": {"base": {"sha": self.base}}}),
        ):
            with self.subTest(event=event_name, payload=event):
                self.classify(event_name, event, SETUP)
        self.classify("workflow_dispatch", {}, ALL)

        product = self.commit({
            "src/main.rs": "fn main() { }\n",
            ".codex/agents/fixture.toml": "fixture = false\n",
        })
        self.push(setup, ALL)
        self.push("0" * 40, ALL)
        self.commit({"AGENTS.md": "Changed fixture instructions\n"})
        self.push(product, SETUP)
        self.classify("pull_request", {"pull_request": {"base": {"sha": self.base}}}, ALL)

    def test_documentation_only_changes_do_not_hide_product_changes(self) -> None:
        docs = self.commit({
            "README.md": "Changed product documentation\n",
            "docs/mockups/page.html": "<p>Mockup</p>\n",
            "liquid/README.md": "Nested documentation\n",
            "AGENTS.md": "Fixture instructions\n",
        })
        for event_name, event in (
            ("push", {"before": self.base}),
            ("push", {"before": "0" * 40}),
            ("pull_request", {"pull_request": {"base": {"sha": self.base}}}),
        ):
            with self.subTest(event=event_name, payload=event):
                self.classify(event_name, event, DOCS)
        self.classify("workflow_dispatch", {}, ALL)

        self.commit({"src/main.rs": "fn main() { }\n"})
        self.push(docs, ALL)
        self.classify("pull_request", {"pull_request": {"base": {"sha": self.base}}}, ALL)

    def test_moving_product_content_into_setup_or_documentation_still_requires_product_gates(
        self,
    ) -> None:
        for source, target, expected in (
            ("src/main.rs", ".agents/main.rs", ALL),
            ("src/main.rs", "docs/main.md", ALL),
            ("liquid/app/lib/main.dart", "docs/main.md", scope(True, "liquid")),
            ("flow-agent/src/lib.rs", "liquid/src/lib.rs", scope(True, "flow_agent", "liquid")),
        ):
            with self.subTest(source=source, target=target):
                self.git("reset", "--quiet", "--hard", self.base)
                before = self.commit({source: "Moved fixture\n"})
                (self.repo / target).parent.mkdir(parents=True, exist_ok=True)
                self.git("mv", source, target)
                self.git("commit", "--quiet", "-m", "Move fixture")
                self.push(before, expected)

    def test_each_product_selects_its_own_and_its_dependents_gates(self) -> None:
        before = self.base
        for name, expected in (
            ("liquid/app/lib/main.dart", scope(True, "liquid")),
            ("liquid/liquid-bridge/rust-toolchain.toml", scope(True, "liquid")),
            ("meta-harness/pi-extension/watershed.ts", scope(True, "meta_harness", "liquid")),
            ("flow-agent/flow-agent-core/src/lib.rs", scope(True, "flow_agent")),
            ("core/core-policy/src/lib.rs", scope(True, "flow_agent")),
            ("proto/proto/src/lib.rs", scope(True, "flow_agent")),
            ("install/install.sh", scope(True, "flow_agent")),
            # A member manifest can change features the others build with.
            ("liquid/liquid-bridge/Cargo.toml", ALL),
            ("Cargo.lock", ALL),
            ("scripts/run-python.mjs", ALL),
            ("tests/test_fixture.py", ALL),
            ("docs/design/tokens.css", ALL),
            ("liquid-tools/helper.mjs", ALL),
            ("flow-agent.toml", ALL),
            ("app/liquid/main.dart", ALL),
        ):
            with self.subTest(path=name):
                current = self.commit({name: "Fixture\n"})
                self.push(before, expected)
                before = current
        self.commit({
            "liquid/app/lib/main.dart": "Changed fixture\n",
            "flow-agent/flow-agent-core/src/lib.rs": "Changed fixture\n",
        })
        self.push(before, scope(True, "flow_agent", "liquid"))

    def test_a_crate_change_selects_every_dependent_crates_gates(self) -> None:
        with (ROOT / "Cargo.toml").open("rb") as manifest:
            workspace = tomllib.load(manifest)["workspace"]
        crates = {}
        for member in workspace["members"]:
            with (ROOT / member / "Cargo.toml").open("rb") as manifest:
                package = tomllib.load(manifest)
            tables = [package, *package.get("target", {}).values()]
            dependencies = {
                name for owner in tables
                for table in ("dependencies", "dev-dependencies", "build-dependencies")
                for name, spec in owner.get(table, {}).items()
                if "path" in (workspace["dependencies"][name]
                              if isinstance(spec, dict) and spec.get("workspace") else spec)
            }
            crates[package["package"]["name"]] = (member, dependencies)

        def dependents(name: str) -> set[str]:
            # Dev-dependencies may form cycles, so expand each crate once.
            found, pending = set(), [name]
            while pending:
                current = pending.pop()
                for other, (_, uses) in crates.items():
                    if current in uses and other not in found:
                        found.add(other)
                        pending.append(other)
            return found - {name}

        outputs = {}
        before = self.base
        for name, (member, _) in crates.items():
            current = self.commit({f"{member}/src/lib.rs": f"// {name}\n"})
            outputs[name] = self.classify("push", {"before": before}, {})
            before = current
        self.assertIn("liquid_bridge", dependents("meta-harness-core"))
        for name, selected in outputs.items():
            for dependent in {name} | dependents(name):
                with self.subTest(crate=name, dependent=dependent):
                    own = {product for product in PRODUCTS if outputs[dependent][product] == "true"}
                    self.assertTrue(own)
                    self.assertTrue(all(selected[product] == "true" for product in own))
                    packages = selected["rust_packages"].split()
                    self.assertTrue(packages == ["--workspace"]
                                    or dependent in packages[1::2], packages)

    def test_exclusions_do_not_cover_similarly_named_paths(self) -> None:
        before = self.base
        for name, expected in (
            ("docs/AGENTS.md", DOCS),
            (".agents.md", DOCS),
            (".codex-tools/helper.mjs", ALL),
            ("docs/tokens.css", ALL),
            ("docs/page.htm", ALL),
            ("docs/page.mdx", ALL),
            ("docs/page.md.rs", ALL),
        ):
            with self.subTest(path=name):
                current = self.commit({name: "Fixture\n"})
                self.push(before, expected)
                before = current

    def test_unavailable_comparison_base_fails_without_a_skip_result(self) -> None:
        self.classify("push", {"before": "f" * 40}, None)


class CiJobScopeTest(unittest.TestCase):
    # Each gated job and the scope output that must be true for it to run.
    JOB_SCOPES = {
        "checks": "docs",
        "test": "product",
        "flow-agent": "flow_agent",
        "m11-evidence": "flow_agent",
        "liquid": "liquid",
    }

    def test_every_job_runs_only_after_classification_of_its_scope(self) -> None:
        workflow = workflow_text()
        self.assertIn("          fetch-depth: 0", step_lines(workflow, "Checkout"))
        self.assertEqual(
            step_run(workflow, "Classify product changes"), "node scripts/ci-product-scope.mjs"
        )
        self.assertIn("        id: scope", step_lines(workflow, "Classify product changes"))
        scope_job = jobs(workflow)["scope"]
        self.assertFalse(any(line.startswith("    if:") for line in scope_job))
        for output in ("docs", "product", *PRODUCTS, "rust_packages"):
            self.assertIn(f"      {output}: ${{{{ steps.scope.outputs.{output} }}}}", scope_job)
        self.assertEqual(list(job_steps(workflow, "scope")),
                         ["Checkout", "Install pinned Node", "Classify product changes"])

        gated = {name: lines for name, lines in jobs(workflow).items() if name not in ("scope", "gates")}
        self.assertEqual(set(gated), set(self.JOB_SCOPES))
        for name, lines in gated.items():
            with self.subTest(job=name):
                self.assertIn("    needs: scope", lines)
                self.assertEqual(
                    [line for line in lines if line.startswith("    if:")],
                    [f"    if: needs.scope.outputs.{self.JOB_SCOPES[name]} == 'true'"],
                )

    def test_required_checks_pass_only_when_every_job_passed_or_was_not_selected(self) -> None:
        workflow = workflow_text()
        gates = jobs(workflow)["gates"]
        self.assertIn(
            "    name: Flow Agent gates (${{ matrix.os }}) [${{ github.event_name }}]", gates
        )
        self.assertIn("        os: [ubuntu-24.04, xcode-27]", gates)
        self.assertIn("    if: always()", gates)
        needs = next(line for line in gates if line.startswith("    needs:"))
        self.assertEqual(
            set(needs.removeprefix("    needs: [").removesuffix("]").split(", ")),
            {"scope", *self.JOB_SCOPES},
        )
        self.assertIn("          NEEDS: ${{ toJSON(needs) }}", step_lines(workflow, "Require every selected job"))
        command = step_run(workflow, "Require every selected job")

        def verdict(results: dict[str, str], outputs: dict[str, str] | None = None) -> bool:
            outputs = ALL if outputs is None else outputs
            needs = {"scope": {"result": results.pop("scope", "success"),
                               "outputs": outputs}}
            needs.update((name, {"result": results.get(name, "success"), "outputs": {}})
                         for name in self.JOB_SCOPES)
            result = subprocess.run(
                [sys.executable, "-c", command], env={**os.environ, "NEEDS": json.dumps(needs)},
                capture_output=True, text=True, timeout=10,
            )
            return result.returncode == 0

        self.assertTrue(verdict({}))
        self.assertTrue(verdict({"flow-agent": "skipped", "m11-evidence": "skipped"},
                                scope(True, "liquid")))
        self.assertTrue(verdict(dict.fromkeys(self.JOB_SCOPES, "skipped"), SETUP))
        # Only the job's own scope output permits its skip; failure is never permitted.
        for name, key in self.JOB_SCOPES.items():
            for other in {*ALL} - {"rust_packages"}:
                with self.subTest(job=name, unselected=other):
                    outputs = ALL | {other: "false"}
                    self.assertEqual(verdict({name: "skipped"}, outputs), other == key)
            for result in ("failure", "cancelled"):
                with self.subTest(job=name, result=result):
                    self.assertFalse(verdict({name: result}, scope(True, "liquid")))
        self.assertFalse(verdict({"scope": "failure"}, {}))
        self.assertFalse(verdict(dict.fromkeys(self.JOB_SCOPES, "skipped") | {"scope": "skipped"}, {}))

if __name__ == "__main__":
    unittest.main()
