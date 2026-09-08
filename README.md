# Watershed

Watershed is an **AGPL/free-software AI-native work platform** for reusable, measurable, and reversible agent workflows. Its independently usable Flow Agent, Meta-Harness, and Liquid layers share one core and protocol; their canonical boundaries and integration model are in [VISION.md](VISION.md).

## Project status

**M1.2 — Flow Agent OS isolation, in progress.** The runtime now implements [trusted Tools with native Flow-file protection](SECURITY.md#accepted-flow-agent-security-target) on Linux x86_64 and macOS ARM64. Native acceptance, installation and repository closeout must pass before this becomes a release claim. Current status is canonical in [PLAN.md](PLAN.md#m12--flow-agent-os-isolation).

[PLATFORMS.md](PLATFORMS.md) defines each product's native release targets, current capabilities and required verification; compilation alone is not a support claim.

## Repo layout

```
core/         core-script (building-block model/parser) and core-policy
              (Tool invocation validation and policy artifacts)
proto/        proto: event and Executor wire schemas/types (the integration seam)
flow-agent/   flow-agent-core (engine/runtime/session), flow-agent-cli
              (human CLI, machine-readable run mode, tail/replay/resume), and
              flow-agent-executor (one-shot native self-protection companion)
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

### Developer/test installation on Linux or macOS

The end-user distribution must follow the [prebuilt-artifact installation contract](SECURITY.md#m12-tool-execution-trust-boundary). Download packaging remains pending. For local staging on a supported native host, build a private bundle from the repository root:

```sh
cargo build --locked --release -p flow-agent-cli -p flow-agent-executor
install_bundle=$(mktemp -d)
install -m 0755 install/install.sh target/release/flow \
  target/release/flow-executor "$install_bundle/"
/bin/sh "$install_bundle/install.sh" --prefix "$HOME/.local/watershed"
"$HOME/.local/watershed/bin/flow" executor check
```

Use a fresh prefix: the installer never upgrades existing binaries. It checks the Default Executor as the intended unprivileged user. Missing host prerequisites are errors; it does not update a kernel, change system security policy or start a privileged service. See [PLATFORMS.md](PLATFORMS.md) for native prerequisites and verification.

Alternatively, omit the Default Executor explicitly and select an administrator-reviewed Custom Executor as the operating-system account that will run Flow:

```sh
/bin/sh "$install_bundle/install.sh" \
  --prefix "$HOME/.local/watershed" --no-default-executor
"$HOME/.local/watershed/bin/flow" executor configure --path /absolute/path/to/custom-executor
"$HOME/.local/watershed/bin/flow" executor check
```

`/bin/sh install/install.sh --help` is the canonical option summary. Custom Executor readiness validates the protocol boundary but is not a compatibility or security certification; see the [Executor architecture](docs/concept/flow-agent-executor-architecture.md).

Set `FLOW_AGENT_HOME` to an unused absolute path before exercising local authoring or runtime state. Workspace layout is illustrated in [`docs/concept/V-Spec_FlowAgent.html`](docs/concept/V-Spec_FlowAgent.html). [`PROTOCOL.md`](PROTOCOL.md) defines Registry authoring; the [registry schema](core/core-script/schemas/registry-block.schema.json) documents its intended field/type shape. Checked-in deterministic examples live under [`flow-agent/fixtures/`](flow-agent/fixtures/) and make no provider, subprocess or isolation claim.

For productive execution, initialize the Global Flow home with `flow init`, configure its provider and model through the V-Spec, inspect authoring grammar with `flow create <tool|instruction|phase|flow> --help`, authenticate through the commands in [PROTOCOL.md](PROTOCOL.md), then run the authored Flow. Engineers configure Tool commands and accepted parameters and trust their complete implementations, helpers and delegation chains. The [security contract](SECURITY.md#m12-tool-execution-trust-boundary) owns the productive boundary.

The Global Flow home and its configuration authority are defined in [PROTOCOL.md](PROTOCOL.md#local-run-storage-and-m11-conversation-trees).

The complete command, storage and Executor contract is in [`PROTOCOL.md`](PROTOCOL.md). Native self-protection is not general filesystem, network or hostile-Tool containment.

## Product boundaries

Sequencing and the MVP project-code VCS boundary are canonical in [PLAN.md](PLAN.md). Surface details live in the [Flow Agent](docs/concept/V-Spec_FlowAgent.html), [Meta-Harness](docs/concept/V-Spec_MetaHarness.html), and [Liquid](docs/concept/V-Spec_Liquid.html) V-Specs; events are defined in [PROTOCOL.md](PROTOCOL.md).

## Start here

- **Why & how it fits together:** [VISION.md](VISION.md)
- **Build plan & milestones:** [PLAN.md](PLAN.md)
- **Current implementation architecture:** [docs/architecture.md](docs/architecture.md)
- **Platform targets and available capabilities:** [PLATFORMS.md](PLATFORMS.md)
- **Executor and Sandbox architecture:** [docs/concept/flow-agent-executor-architecture.md](docs/concept/flow-agent-executor-architecture.md)
- **Rules for AI/human contributors:** [AGENTS.md](AGENTS.md)
- **Open decisions (human decision page):** [docs/decisions/open-decisions.html](docs/decisions/open-decisions.html)
- **Terminology:** [GLOSSARY.md](GLOSSARY.md)

## License

Watershed-authored files are free software, licensed under the GNU Affero General Public License, version 3 (SPDX-License-Identifier: `AGPL-3.0-only`) unless otherwise stated. The full license text is in [LICENSE](LICENSE). Vendored third-party material retains its own license and is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The project's posture is transparency, self-hostability and user freedom; there are no proprietary tiers or open-core commercialization claims in these docs.

Copyright (C) 2026 Open-Equilibrium. Project owner: **Open-Equilibrium**. Contributions are accepted under the **Developer Certificate of Origin** (DCO); no CLA is required. See [CONTRIBUTING.md](CONTRIBUTING.md).
