# Watershed

Watershed is an **AGPL/free-software AI-native work platform** for reusable, measurable, and reversible agent workflows. Its independently usable Flow Agent, Meta-Harness, and Liquid layers share one core and protocol; their canonical boundaries and integration model are in [VISION.md](VISION.md).

## Project status

**M1 Flow Agent MVP stage.** This repository contains the standalone Flow Agent CLI runtime, protocol/event contracts, deterministic fixture streams, policy artifacts, sandbox-negative tests and M1 validation gates described in [PLAN.md](PLAN.md).

## Repo layout

```
core/         core-script (building-block model/parser) and core-policy
              (capability model + policy→sandbox compiler)
proto/        proto: event schema and serialization (the integration seam)
flow-agent/   flow-agent-core (engine/runtime/session) and flow-agent-cli
              (human CLI, machine-readable run mode, tail/replay/resume)
meta-harness/ host-scoped headless control plane for local CLI agents
liquid/       local-first Page/Block workspace and app-building product
docs/         governance, specs, decisions
```

## Build and run Flow Agent

From the repo root:

```console
cargo build --workspace
cargo test --workspace
```

Run the checked-in smoke fixture from its workspace directory:

```console
cd flow-agent/fixtures/smoke-loop
cargo run -p flow-agent-cli -- run smoke-loop --emit jsonl
cargo run -p flow-agent-cli -- replay smoke-loop --emit jsonl
cargo run -p flow-agent-cli -- tail smoke-loop --emit jsonl --no-follow
cargo run -p flow-agent-cli -- sessions
```

Workspace layout and registry fields are defined in [`docs/concept/V-Spec_FlowAgent.html`](docs/concept/V-Spec_FlowAgent.html); checked-in examples live under [`flow-agent/fixtures/`](flow-agent/fixtures/).

## Product boundaries

Sequencing and the MVP project-code VCS boundary are canonical in [PLAN.md](PLAN.md). Surface details live in the [Flow Agent](docs/concept/V-Spec_FlowAgent.html), [Meta-Harness](docs/concept/V-Spec_MetaHarness.html), and [Liquid](docs/concept/V-Spec_Liquid.html) V-Specs; events are defined in [PROTOCOL.md](PROTOCOL.md).

## Start here

- **Why & how it fits together:** [VISION.md](VISION.md)
- **Build plan & milestones:** [PLAN.md](PLAN.md)
- **Rules for AI/human contributors:** [AGENTS.md](AGENTS.md)
- **Open decisions (human decision page):** `docs/decisions/open-decisions.html`
- **Terminology:** [GLOSSARY.md](GLOSSARY.md)

## License

Watershed-authored files are free software, licensed under the GNU Affero General Public License, version 3 (SPDX-License-Identifier: `AGPL-3.0-only`) unless otherwise stated. The full license text is in [LICENSE](LICENSE). Vendored third-party material retains its own license and is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The project's posture is transparency, self-hostability and user freedom; there are no proprietary tiers or open-core commercialization claims in these docs.

Copyright (C) 2026 Open-Equilibrium. Project owner: **Open-Equilibrium**. Contributions are accepted under the **Developer Certificate of Origin** (DCO); no CLA is required. See [CONTRIBUTING.md](CONTRIBUTING.md).
