# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance targets are canonical in `PERFORMANCE.md`.

Created: 2026-06-05

## MVP boundary

The first MVP is **Loop Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. Project-code-history/VCS questions are deferred until after the Loop Agent + Meta-Harness MVPs validate the core workflow. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3).

## Sequencing and adoption

Build the shared substrate, then Loop Agent, Meta-Harness and Liquid. This dependency/order-of-risk sequence validates execution, then multi-agent governance, then the integrated workspace without letting the broadest surface block earlier validation. All three remain independently usable layers of one platform; integration is canonical in `VISION.md`.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Loop Agent and Meta-Harness wedges are validated.

## Milestones

### M0 — Loop Agent MVP implementation packet + walking skeleton

**Wedge:** Loop Agent execution wedge (developer/open-source credibility).

**Purpose:** establish the implementation packet and walking skeleton needed to build the standalone Loop Agent MVP without inventing architecture.

**Deliverables:**

- Rust workspace and the `core`, `proto`, `loop-agent`, `meta-harness`, and `liquid` scaffold.
- Versioned building-block, event, runtime, session, policy, and sandbox contracts. Canonical owners: `PROTOCOL.md`, `SECURITY.md`, and the Loop Agent V-Spec.
- Deterministic fixtures and expected streams per `TESTING.md` (ADR-0034).
- Cross-platform CI, dependency, coverage, link, and render gates per `TESTING.md`, `SECURITY.md`, and `.github/workflows/ci.yml`.

**Decision state:** M0 and M1 are unblocked; accepted decisions are in [`ADR-LOG.md`](docs/adr/ADR-LOG.md).

**DoD:** the scaffold compiles on Linux, macOS, and Windows; its canonical contracts and fixtures are sufficient to implement M1 without architectural guesses; and all M0 gates defined by the canonical test, security, and CI sources pass.

### M1 — Loop Agent MVP (standalone CLI)

**Wedge:** Loop Agent execution wedge — prove evented agent loops as a deterministic, auditable, reusable agent-loop runtime (not a generic coding agent).

**Status:** M1 implementation is in progress. The standalone CLI runtime, JSONL event stream, local session log, replay/tail/resume commands, fixture registry loading and validation gates are in active hardening against the DoD.

**Deliverables:**

- Standalone CLI Loop Agent (human CLI run path).
- Headless JSONL event stream over stdout.
- Local append-only session/transcript log (ADR-0037); initial resume/tail/replay behavior over the log.
- Public runtime events persisted before bounded non-blocking live notification, with caller-owned sequence replay from the authoritative log (ADR-0036, ADR-0059, ADR-0062).
- Building-block registry for Tools, Instructions, Phases, Loops and Connections using explicit by-name/id references, canonical serialization and cycle detection (ADR-0031).
- Deterministic FSM phase/step engine: phase order, available tools, instruction loading and state transitions are deterministic; LLM/tool outputs are inputs to deterministic transitions.
- Deterministic, cache-stable `loop-context-v0` compilation over mandatory active scope plus narrowly bounded continuity, with reproducible per-turn manifests; persisted compaction and retrieval are post-M1 (ADR-0050, ADR-0058).
- Script-defined Tools/Instructions/Phases/Loops with recursive composition (`Loop` as a building block).
- Event-driven execution: no polling loop for normal agent progress.
- Runtime kernel: deterministic bounded in-process fixture interpretation plus session event and context-manifest logs. External subprocess timeouts, bounded stdout/stderr, per-tool run logs and `tool.timed_out` remain post-M1.
- Deterministic in-process enforcement/emulation for declared command, parameter-schema, read/write, protected-path and deny-all network capabilities per loop. Linux-target policy rejects non-empty network allowlists; Linux Landlock/seccomp OS enforcement and macOS Seatbelt parity are post-M1 targets (ADR-0051, ADR-0052).
- Protocol adapter that emits normalized `proto` v0 events.
- Golden loops and sandbox-negative tests per `TESTING.md` (ADR-0034).

**DoD:** a multi-phase local loop with a subloop runs headless from the CLI; compiles deterministic, budget-safe provider context and manifests; persists every canonical event before any live notification; emits the expected JSONL stream; persists/replays/tails/resumes the local session log; enforces phase/tool scoping; writes session event and context-manifest logs; and passes context, FSM, event-ordering, transcript-persistence and sandbox-negative policy-emulation tests (with macOS policy-artifact parity checks). It also meets the `TESTING.md` coverage gate (ADR-0022/ADR-0060) and all Loop Agent M1 budgets in `PERFORMANCE.md`. Loop Agent runs standalone with no dependency on Meta-Harness or Liquid, and no Loop Agent MVP feature depends on a Watershed project-history/VCS engine.

### M2 — Meta-Harness MVP + AgentPulse

**Wedge:** Meta-Harness team/control/governance wedge — turn Loop Agent and external agents into a controllable, observable, measurable system. Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained, host-scoped headless control plane** with CLI/API/service surfaces — usable without Liquid. Each instance controls only CLI agents on its own host. Its public API remains transport-neutral; D-023 selects the local and authenticated remote bindings. Liquid integrates later as a client of one or more instances. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI (headless user/admin: run/session/config/metrics commands).
- Local service/daemon shape (sidecar for Liquid or standalone daemon) with the transport-neutral API and D-023 bindings.
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, agent schedule/trigger control, AgentPulse queries, approval/reject/revert (transport: D-023).
- Central configuration model that resolves shared Watershed building blocks to the correct agent CLI (Loop Agent, Codex CLI, Claude Code, Pi Agent, etc.).
- Control plane: session registry, routing, task state, attention state and agent schedule/trigger skeleton.
- Host-local executor that rejects cross-host agent-process control.
- Adapters: Loop Agent (via its public runtime surfaces) + at least one external CLI adapter.
- Event/transcript ingestion from agents; artifact/log/handoff indexing (logs, structured summaries, host-provided diffs, handoff packs, checkpoints).
- AgentPulse v0 metrics: rework ratio, first-attempt success and cost-per-productive-outcome using formulas decided before M2 implementation; computed and stored by Meta-Harness and queryable through CLI/API.
- Policy-gated configuration writes with audit trail and review flow.
- **No rich standalone GUI**; M2 packaging, including whether it adds a small status screen after the headless controller works, remains D-021.

**DoD:** monitor, steer and configure at least two different CLI agents on the Meta-Harness host from one control surface, with both represented through one normalized session/event model; reject attempts to claim or control a process on another host; run without Liquid; expose the public API/protocol through the bindings selected by D-023; integrate Loop Agent through its public runtime surfaces (not its internals); resolve shared config without duplicated per-agent config directories for the same capability; report decided AgentPulse v0 metrics through CLI/API; and require approval plus an audit record for every sensitive config change.

### M3 — Liquid (staged standalone workspace product)

**Wedge:** prove safe human/agent co-editing of local-first Workspace state with attributed, reviewable and reversible History. Liquid remains useful without agents, network or a hosted service. Do not build a generic Notion clone.

The target product is canonical in [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html). M3 is staged so workspace fundamentals prove value before sync, community extensions and broad integrations.

#### M3a — Local Workspace foundation

- Local replica as the only interactive store; storage choice remains D-029.
- Pages, Blocks, Views, Sources, Connections, Automations, Roles, History and settings.
- Explorer with Pages and Sources as peer navigation entries; Page templates create use-case compositions rather than new Page Types.
- PowerBar for finding Workspace objects, creating Blocks and invoking permitted Workspace actions.
- Blank Page and deterministic first-action creation: typing creates a Text Block, `/` opens the Block chooser, paste/drop/drawing select matching Blocks, and text shortcuts format inside Text Blocks.
- Content-first flow followed by explicit responsive Arrange mode. Mobile retains every Block; complex Views open focused surfaces. Safe compact OS widgets remain post-MVP.
- Built-in Text, Database, Chart, Code, Formula, Media/Embed and Whiteboard Blocks.
- Synchronized multi-Page Views over one canonical Block. Duplicate creates a new Block; deleting the last View deletes the Block, its Connections and owned Automations in one reversible Action, while independent dependent Automations are disabled for repair.
- Connections/formulas for direct logic and Workspace Automations for when/if/then logic.
- Allow-only Roles with default-denied unlisted access; explicit deny rules are post-MVP.
- One mutation pipeline and History for every M3a write surface, including UI, Connections/formulas, Automations and import. History design and revert detail remain D-028/D-031.

**M3a DoD:** offline, a user can create and navigate Pages/Sources, act immediately on a blank Page, edit and connect Blocks, reuse a Block through synchronized Views, arrange the Page responsively, assign a Role, run a simple Automation and recover the tested deletion cascade through History.

#### M3b — Apps and agent actions

- Restricted local JavaScript/TypeScript App Runtime with declarative UI, resource limits and explicit capabilities; WASM is a later extension.
- App and Agent Session Blocks, including permitted agent actions in the PowerBar.
- App SDK for AI/user-built Workspace-local Apps, with versioned state, Views and typed App actions.
- Workspace CLI/API (D-027) exposes compact typed reads, mutation proposals/execution and permitted App actions to agents.
- Liquid AI uses the same Role, capability, mutation and History boundaries.
- Liquid AI, CLI/API, App and Meta-Harness integrations join the same permissioned mutation/History pipeline.
- Optional Meta-Harness projections show friendly harness/config/location/session choices while preserving instance identity, freshness and authority. Live commands route to the owning instance.

**M3b DoD:** a user can prompt Liquid AI to build an interactive App over connected Block data, use it without build/deploy/CI knowledge, inspect or restore its versions through History, share it within the Workspace and allow an agent to invoke only selected App actions through the CLI/API.

#### M3c — Central sync, mobile and headless execution

- Central Sync Server with resumable star-topology exchange; conflict rules remain D-035.
- Authorized user devices sync complete Workspaces and continue offline from local replicas.
- Optional headless Liquid replica receives only explicitly enabled Workspaces and exposes the same Role/mutation boundary to same-host agents while user devices are offline.
- Sync applies received actions through the same permissioned mutation/History pipeline.
- Sync, headless Liquid and Meta-Harness remain separate logical roles even when one hosted deployment co-locates them.

**M3c DoD:** laptop, phone and an opted-in headless replica converge through the Sync Server; disconnecting any replica never blocks its local work; reconnection resumes without silent loss; a server agent can act only within its Workspace and Role.

#### M3d — Extension ecosystem and MCP adapters

- Versioned Block SDK and signed, sandboxed Block Registry for reusable community Block Types; no arbitrary third-party native code.
- MCP adapter maps external MCP capabilities to the same typed App actions and Role/capability boundary, with or without a visible App View.
- Import/export paths let integrations become Apps for users and compact CLI actions for agents without exposing MCP concepts in normal UX.

**M3d DoD:** an installed Block Registry package is isolated, upgradeable and responsive; an external MCP integration can be permissioned once, represented as an App or headless integration, and invoked through the same action contract from Liquid or an authorized agent.
