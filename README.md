# Watershed

Watershed is an **AGPL/free-software AI-native work platform** for reusable, measurable, and reversible agent workflows. Its independently usable Flow Agent, Meta-Harness, and Liquid layers share one core and protocol; their canonical boundaries and integration model are in [VISION.md](VISION.md).

## Project status

**M1.2 — Flow Agent OS isolation.** Current milestone status is canonical in [PLAN.md](PLAN.md#m12--flow-agent-os-isolation). Productive Tools use the one-shot Executor boundary; Ubuntu 24.04 x64 is the only productive platform, including for Custom Executors, and other platforms fail closed.

## Repo layout

```
core/         core-script (building-block model/parser) and core-policy
              (capability model + policy→sandbox compiler)
proto/        proto: event and Executor wire schemas/types (the integration seam)
flow-agent/   flow-agent-core (engine/runtime/session), flow-agent-cli
              (human CLI, machine-readable run mode, tail/replay/resume), and
              flow-agent-executor (one-shot Ubuntu isolation companion)
meta-harness/ host-scoped headless control plane for local CLI agents
liquid/       local-first Page/Block workspace and app-building product
docs/         governance, specs, decisions
```

Current crate dependencies and major Flow Agent responsibility paths are mapped in [`docs/architecture.md`](docs/architecture.md).

## Build and run Flow Agent

From the repo root:

```console
cargo build --locked --workspace
cargo nextest run --config 'target."cfg(all())".runner = ["node", "../../scripts/run-isolated-rust-test.mjs"]' --locked --workspace --all-targets
```

### Install productive Flow Agent on Ubuntu 24.04 x64

From the repository root, build and stage one administrator-owned bundle. The installer requires `install.sh`, `flow` and, for the standard path, the static `flow-executor` as regular executable siblings. It installs into an unused absolute prefix and does not perform upgrades.

```sh
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  apparmor bubblewrap build-essential musl-tools procps util-linux
rustup target add x86_64-unknown-linux-musl
cargo build --locked --release -p flow-agent-cli --bin flow
cargo build --locked --release -p flow-agent-executor --bin flow-executor \
  --target x86_64-unknown-linux-musl

install_bundle=$(sudo mktemp -d /var/tmp/watershed-install.XXXXXX)
sudo chmod 0755 "$install_bundle"
sudo install -m 0755 install/install.sh "$install_bundle/install.sh"
sudo install -m 0755 target/release/flow "$install_bundle/flow"
sudo install -m 0755 \
  target/x86_64-unknown-linux-musl/release/flow-executor \
  "$install_bundle/flow-executor"
```

Choose one installation path. The standard path installs and checks the bundled Default Executor. On Ubuntu 24.04, authorize its unprivileged Bubblewrap user namespace once:

```sh
sudo tee /etc/apparmor.d/watershed-bwrap-userns >/dev/null <<'APPARMOR'
abi <abi/4.0>,
include <tunables/global>

/usr/bin/bwrap flags=(unconfined) {
  userns,
}
APPARMOR
sudo apparmor_parser --replace /etc/apparmor.d/watershed-bwrap-userns
```

```sh
sudo /bin/sh "$install_bundle/install.sh" --prefix /opt/watershed
/opt/watershed/bin/flow executor check
```

Run the command from the unprivileged operating-system account that will run Flow. The installer verifies the Default Executor as that account; if it fails, do not disable the kernel restriction.

The custom path explicitly omits the Default Executor. After installation, the operating-system account that will run Flow Agent selects an administrator-reviewed absolute Custom Executor and checks it; do not use `sudo` for these two configuration commands unless that account is root.

```sh
sudo /bin/sh "$install_bundle/install.sh" \
  --prefix /opt/watershed --no-default-executor
/opt/watershed/bin/flow executor configure --path /absolute/path/to/custom-executor
/opt/watershed/bin/flow executor check
```

`/bin/sh install/install.sh --help` is the canonical option summary. Custom Executor readiness validates the protocol boundary but is not a compatibility or security certification; see the [Executor architecture](docs/concept/flow-agent-executor-architecture.md).

Set `FLOW_AGENT_HOME` to an unused absolute path before exercising local authoring or runtime state. Workspace layout is illustrated in [`docs/concept/V-Spec_FlowAgent.html`](docs/concept/V-Spec_FlowAgent.html). [`PROTOCOL.md`](PROTOCOL.md) defines Registry authoring; the [registry schema](core/core-script/schemas/registry-block.schema.json) documents its intended field/type shape. Checked-in deterministic examples live under [`flow-agent/fixtures/`](flow-agent/fixtures/) and make no provider, subprocess or isolation claim.

For productive execution, initialize the Global Flow home with `flow init`, configure its provider and model through the V-Spec, inspect authoring grammar with `flow create <tool|instruction|phase|flow> --help`, authenticate through the commands in [PROTOCOL.md](PROTOCOL.md), then run the authored Flow. The standard Ubuntu installation resolves its sibling `flow-executor`; `flow executor check` reports readiness. Agentic Engineers define each Flow's Tools, exact mounts and runtime-read profile; other users may run those predefined Flows without gaining an escalation surface. The [security contract](SECURITY.md#m12-tool-execution-trust-boundary) owns the productive boundary.

`FLOW_AGENT_HOME` defaults to `~/.flow` on Unix and `%USERPROFILE%\.flow` on Windows. Its `config.yaml` and registry are the sole implicit technical authority. Workspace `.flow` content is not discovered; optional global-home and harness-start Workspace `AGENTS.md` files provide instructions only.

The complete command, storage and Executor contract is in [`PROTOCOL.md`](PROTOCOL.md). Productive Tool networking is deny-all; positive grants remain deferred.

## Product boundaries

Sequencing and the MVP project-code VCS boundary are canonical in [PLAN.md](PLAN.md). Surface details live in the [Flow Agent](docs/concept/V-Spec_FlowAgent.html), [Meta-Harness](docs/concept/V-Spec_MetaHarness.html), and [Liquid](docs/concept/V-Spec_Liquid.html) V-Specs; events are defined in [PROTOCOL.md](PROTOCOL.md).

## Start here

- **Why & how it fits together:** [VISION.md](VISION.md)
- **Build plan & milestones:** [PLAN.md](PLAN.md)
- **Current implementation architecture:** [docs/architecture.md](docs/architecture.md)
- **Executor and Sandbox architecture:** [docs/concept/flow-agent-executor-architecture.md](docs/concept/flow-agent-executor-architecture.md)
- **Rules for AI/human contributors:** [AGENTS.md](AGENTS.md)
- **Open decisions (human decision page):** [docs/decisions/open-decisions.html](docs/decisions/open-decisions.html)
- **Terminology:** [GLOSSARY.md](GLOSSARY.md)

## License

Watershed-authored files are free software, licensed under the GNU Affero General Public License, version 3 (SPDX-License-Identifier: `AGPL-3.0-only`) unless otherwise stated. The full license text is in [LICENSE](LICENSE). Vendored third-party material retains its own license and is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The project's posture is transparency, self-hostability and user freedom; there are no proprietary tiers or open-core commercialization claims in these docs.

Copyright (C) 2026 Open-Equilibrium. Project owner: **Open-Equilibrium**. Contributions are accepted under the **Developer Certificate of Origin** (DCO); no CLA is required. See [CONTRIBUTING.md](CONTRIBUTING.md).
