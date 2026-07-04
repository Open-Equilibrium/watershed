# Watershed

Watershed is an **AGPL/free-software AI-native work platform** for reusable, measurable, and reversible agent workflows. It is one platform with **three independently usable layers**, implemented as a monorepo over a shared core and a single versioned protocol — not three unrelated products and not a monolith.

| Platform layer   | Product surface | Standalone job                                                            | Integrated value                                                                  |
| ---------------- | --------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Execution        | Loop Agent      | Run repeatable, auditable AI-agent workflows from the CLI.                | Emits structured loop events and transcripts for Meta-Harness/Liquid.             |
| Control          | Meta-Harness    | Control, observe, measure, and govern many agents.                        | Normalizes sessions/config/metrics across Loop Agent and external agents.         |
| Workspace/action | Liquid          | Let humans and agents safely co-edit a workspace with reversible history. | Gives users a visual workspace where agent actions are reviewable and revertible. |

## Project status

**M1 Loop Agent MVP stage.** This repository contains the standalone Loop Agent CLI runtime, protocol/event contracts, deterministic fixture streams, policy artifacts, sandbox-negative tests and M1 validation gates described in [PLAN.md](PLAN.md).

Each layer has independent value; the combined platform is stronger than any single layer. The defensible idea is the **combination** of structured loops, agent control, measurable outcomes, and reversible agent-edited workspace state — built as transparent, self-hostable, AGPL-licensed infrastructure rather than a proprietary/open-core product.

## Adoption wedges

Watershed is built and adopted through its layers in order of risk, as one platform:

- **Loop Agent — developer/open-source execution wedge.** Prove deterministic, reusable, evented agent loops and earn developer trust with a concrete artifact that can be run, inspected, forked, and shared.
- **Meta-Harness — team/control/governance wedge.** Turn Loop Agent and external agents into an observable, measurable, governable system through open, self-hostable control.
- **Liquid — long-term workspace/action wedge.** Prove safe human/agent workspace co-editing with attributed, reviewable, reversible action history.

The layers integrate through public surfaces, but each remains independently usable. See [PLAN.md](PLAN.md) for the wedge sequencing and [VISION.md](VISION.md) for the integration model.

## Repo layout

```
core/         core-script (building-block model/parser) and core-policy
              (capability model + policy→sandbox compiler)
proto/        proto: event schema and serialization (the integration seam)
loop-agent/   loop-agent-core (engine/runtime/session) and loop-agent-cli
              (human CLI, machine-readable run mode, tail/replay/resume)
meta-harness/ control + analytics service
liquid/       the UI surface that composes everything
docs/         governance, specs, decisions
```

## Build and run Loop Agent

From the repo root:

```powershell
cargo build --workspace
cargo test --workspace
```

Run the checked-in smoke fixture from its workspace directory:

```powershell
cd loop-agent/fixtures/smoke-loop
cargo run -p loop-agent-cli -- run smoke-loop --emit jsonl
cargo run -p loop-agent-cli -- replay smoke001 --emit jsonl
cargo run -p loop-agent-cli -- tail smoke001 --emit jsonl --no-follow
cargo run -p loop-agent-cli -- sessions
```

A Loop Agent workspace has this M1 layout:

```text
.loop/config.yaml                         workspace config
registry/{tools,instructions,phases,loops,connections}/
.loop/sessions/<session_id>.jsonl         runtime event log
.loop/sessions/<session_id>.lock          active-session lock
.loop/logs/<session_id>.log               resume metadata sidecar
out/                                      fixture/runtime output
```

The minimal `.loop/config.yaml` shape used by the fixtures is:

```yaml
fixture_profile: stub-model
registry_root: registry
stub_model: deterministic
```

Registry block fields are defined in [`docs/concept/V-Spec_LoopAgent.html`](docs/concept/V-Spec_LoopAgent.html); the checked-in examples live under [`loop-agent/fixtures/`](loop-agent/fixtures/).

## Loop Agent is a standalone product

Loop Agent is a **standalone CLI agent product first**, with Pi-style runtime integration surfaces: a human CLI, a headless JSONL event stream, designed-for remote-control/embeddable seams, and its own local session/transcript store. Meta-Harness and Liquid integrate with Loop Agent through its public runtime surfaces; **they are not required to run Loop Agent.** See [`docs/concept/V-Spec_LoopAgent.html`](docs/concept/V-Spec_LoopAgent.html) for the surfaces and [`PROTOCOL.md`](PROTOCOL.md) for the event contract.

## Meta-Harness is a self-contained control plane

Meta-Harness is a **self-contained headless control plane** for many agents. It can be used directly through CLI/API/service mode — CI, servers, power users and external (BYOA) agents drive it headlessly — and **can run without Liquid**. Liquid is the _primary rich UI_ that consumes it, not a prerequisite; Meta-Harness does not ship a competing full GUI in the MVP. It owns the session registry, adapter model, central config resolution, scheduling/automations, artifact indexing and the AgentPulse engine; Liquid renders these. See [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

## Liquid is a standalone workspace product

Liquid is a **standalone native workspace and app-building product**. Users compose dashboards, views, components, scripts, data sources and automations into custom workflows across desktop and mobile, with local workspace storage, an internal **workspace action history / VCS** and a **workspace CLI/API** that lets external agents and tools read and edit workspace data through a permissioned, fully-recorded mutation pipeline. **Liquid remains useful with neither Loop Agent nor Meta-Harness installed;** they integrate as _optional_ runtime/control-plane providers. Liquid's workspace action history is a workspace VCS over Liquid's own data — **not** a project-code VCS. See [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html).

## MVP scope

Loop Agent's MVP runs as a CLI inside normal Git projects. Watershed does **not** include a dedicated **project-code** VCS/history engine in the MVP, and Loop Agent does not own VCS behavior. Loop Agent's local session/transcript store is runtime state, not a project VCS/history engine. Those questions are deferred until after the Loop Agent and Meta-Harness MVPs prove the core workflow. (This is separate from Liquid's internal workspace action history, which is part of Liquid's product scope.)

## Start here

- **Why & how it fits together:** [VISION.md](VISION.md)
- **Build plan & milestones:** [PLAN.md](PLAN.md)
- **Rules for AI/human contributors:** [AGENTS.md](AGENTS.md)
- **Open decisions (human dashboard):** `docs/decisions/open-decisions.html`
- **Terminology:** [GLOSSARY.md](GLOSSARY.md)

## License

Watershed-authored files are free software, licensed under the GNU Affero General Public License, version 3 (SPDX-License-Identifier: `AGPL-3.0-only`) unless otherwise stated. The full license text is in [LICENSE](LICENSE). Vendored third-party material retains its own license and is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The project's posture is transparency, self-hostability and user freedom; there are no proprietary tiers or open-core commercialization claims in these docs.

Copyright (C) 2026 Open-Equilibrium. Project owner: **Open-Equilibrium**. Contributions are accepted under the **Developer Certificate of Origin** (DCO); no CLA is required. See [CONTRIBUTING.md](CONTRIBUTING.md).
