# Vision

## Thesis

AI-agent work should be **structured, observable, measurable, and safely reversible**. Watershed is one **AGPL/free-software AI-native work platform** that makes agent workflows **reusable, measurable, and reversible** for technical teams. It is not three unrelated products and not a monolith: it is one platform with three independently usable layers, integrated over a shared core and a single protocol.

```text
Execution layer:         Loop Agent
Control layer:           Meta-Harness
Workspace/action layer:  Liquid
```

The platform value chain:

```text
Loop Agent makes agent work repeatable.
Meta-Harness makes agent work observable and governable.
Liquid makes agent changes visible, editable, and reversible in a workspace.
```

This file describes **how the layers integrate and which features emerge only from that integration.** Per-layer internals live in the V-Spec concept files; terms are defined in `GLOSSARY.md`.

## Standalone layers, compound platform value

Each layer must stand on its own, and the integrated platform must be worth more than the sum of the layers:

- **Loop Agent** must be useful without Meta-Harness or Liquid.
- **Meta-Harness** must be useful without Liquid.
- **Liquid** must be useful without Loop Agent or Meta-Harness.
- The integrated platform adds value the layers cannot reach alone:
  - Loop Agent emits structured runtime events.
  - Meta-Harness normalizes control, config, metrics, sessions and transcripts across agents.
  - Liquid renders and stores human/agent workspace actions.
  - Agent changes are permissioned, attributed, reviewable and revertible.

Because Watershed is AGPL/free software, the platform emphasizes **transparency, self-hostability, user freedom and inspectable behavior**: users can read, run, self-host, fork and verify core behavior. This is a public-good, community-trust posture, **not** a proprietary/open-core monetization play (license posture: ADR-0019 in `docs/adr/ADR-LOG.md`).

## Standalone jobs-to-be-done

- **Loop Agent:** *Run repeatable, auditable AI-agent workflows from my CLI.*
- **Meta-Harness:** *Control, observe, measure, and govern many agents.*
- **Liquid:** *Let humans and agents safely co-edit a workspace with reversible history.*

## Integration model: shared core, host-scoped execution

Watershed is a monorepo, **not** a monolith. The tools share `core` (building-block/script format, identity/permissions, policy→sandbox compiler and configuration helpers) and talk over one versioned **protocol** (`proto`). Execution ownership stays local to each host; APIs and workspace sync may cross hosts. Each tool stays independently runnable:

- **Loop Agent** is a **standalone CLI agent product** (CLI-only, local). It is usable on its own — by humans, by scripts/CI, and later as an embeddable core library — and exposes CLI run/replay/tail/resume plus a JSONL event stream; RPC/control, export and embedding are later seams. Per-product detail: [`docs/concept/V-Spec_LoopAgent.html`](docs/concept/V-Spec_LoopAgent.html).
- **Meta-Harness** is a **self-contained, host-scoped headless control plane** over the CLI agents running on its own host (Loop Agent + adapters for external agents). It centralizes configuration, runs a session registry, schedules and automations, persists its own state/audit trail and computes AgentPulse. Its CLI/API/service may be reached locally or remotely, but one Meta-Harness never owns or launches agent processes on another host. Per-product detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).
- **Liquid** is a **standalone local-first workspace and app-building product** built from Pages, Blocks, Block Views, Sources and explicit Block Connections. It always reads and writes a local workspace replica through its action-history pipeline. An optional sync host exchanges committed workspace changes opportunistically and resumably; loss of connectivity never replaces the local workspace with a remote mode. Liquid can present projections from one or more Meta-Harness instances in one UI while preserving instance identity and command authority. Per-product detail: [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html).

**Loop Agent is a standalone product, not a backend.** A local Meta-Harness may launch and observe it through its public runtime surfaces (CLI and JSONL now; RPC/control, export and embedding when implemented). No other layer reads Loop Agent's local `.loop/sessions` store directly, and neither Meta-Harness nor Liquid is a prerequisite for using Loop Agent. Liquid uses Meta-Harness as its single agent-control path instead of owning CLI processes itself.

**Meta-Harness and Liquid are architecturally separate but integrated.** Meta-Harness can run without Liquid (headless, CI, server, BYOA). Liquid presents session/config/metric/automation surfaces from one or more Meta-Harness instances instead of duplicating their backends, and owns the rich Page/Block UI and PowerBar. A unified Liquid projection is **one pane of glass, not one fictional Meta-Harness**: cached records retain source instance, freshness and authority, and each command returns to the instance that owns the target. Meta-Harness does not own UI, reach into Loop Agent internals or act as a project VCS/history engine.

**Liquid is a standalone product, not merely a UI.** It is useful with no agents installed; the recommended first scope is Pages containing text, databases, charts, local scripts and other Blocks, plus LLM-assisted app building (D-026). External agents and tools reach Liquid through its workspace CLI/API; every workspace mutation — human, Liquid AI, sync or external agent — flows through one permissioned pipeline and is recorded in Liquid's internal action history so changes can be reviewed and reverted. Liquid's workspace action history is a workspace VCS over Liquid's own data, **not** a project-code VCS, and Loop Agent/Meta-Harness never mutate Liquid storage directly.

Mental model: editor + Language Server Protocol. Liquid is the local-first editor; each Meta-Harness is a host-scoped server; `proto` is the seam. This yields one UX without hiding ownership or coupling the tools.

Watershed's defensible trust model is the combination:

```text
structured loops
+ scoped runtime capabilities
+ normalized events/transcripts
+ policy gates
+ metric feedback
+ permissioned workspace mutations
+ action history/revert
+ AGPL/free-software transparency
```

Self-hostability and inspectability are part of this AGPL-aligned trust model; see `SECURITY.md` for the cross-cutting safety stance.

## Positioning: what Watershed is and is not

Do **not** position Watershed as: another generic coding agent; another Notion clone; three unrelated products; a status UI for Loop Agent only; or a proprietary/open-core monetization play with paid-tier assumptions.

Position Watershed as: reusable, measurable, reversible AI-agent workflows; structured agent execution + a neutral control plane + safe workspace mutation; one AGPL/free-software platform with independently usable layers; transparent, self-hostable infrastructure for agentic work. Concretely, Loop Agent is not "just a worker," Meta-Harness is not "just a backend," and Liquid is not "just the UI" — each is independently useful and none is merely a part of another.

## MVP boundary: no project-history engine
The MVP works inside normal Git projects, but Watershed does **not** own project VCS behavior and does not introduce a dedicated project-history engine. The former VCS/history questions are deferred until after the Loop Agent and Meta-Harness MVPs validate the core workflow. Until then, auditability comes from structured events, logs, config snapshots and the host project's normal Git workflow where applicable.

## Features that exist only through integration
1. **Run Loops from anywhere.** Start a loop from Liquid's PowerBar, have Meta-Harness schedule it, or let a BYOA client start it while mobile. The command goes to the Meta-Harness on the agent's host; offline Liquid can show cached state but cannot pretend live control succeeded.
2. **One configuration model across agents.** Building blocks, instructions, tools, loops, schedules and connections are defined once and resolved to the proper target CLI (Loop Agent, Codex CLI, Claude Code, Pi Agent, etc.).
3. **Meta-Agent with safe configuration access.** A Meta-Agent (Liquid-native or BYOA) can read, monitor, evaluate **and reconfigure** underlying agents; sensitive changes are policy-gated, audited and reviewable.
4. **Measurement, not just throughput.** AgentPulse (rework ratio, first-attempt success, cost-per-productive-outcome) belongs to Meta-Harness; its metrics render through Liquid Blocks and their Views.
5. **Loops as AI-native processes.** Loops are event-driven process definitions with deterministic *orchestration* (not output): runs can be measured, compared and optimized instead of tuning one generalized agent setup.
6. **Loops as data consumers and producers.** Through permissioned Liquid APIs, loops can read or update Block data and external Sources, covering automation use cases without direct storage access.
7. **Building blocks travel.** A Tool, Instruction, Phase or Loop defined once is reusable across loops and agents; Liquid can expose or invoke it through a Block without duplicating agent configuration.

## Non-goals
- Not a personal assistant with a persona; not a one-click out-of-the-box harness.
- Liquid is an OS-abstracting **app**, not an operating system.
- The MVP does not replace Git or implement a project VCS/history system.
