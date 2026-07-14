# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance targets are canonical in `PERFORMANCE.md`.

Created: 2026-06-05
Updated: 2026-07-14

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
- Deterministic D-015 fixtures and expected streams per `TESTING.md`.
- Cross-platform CI, dependency, coverage, link, and render gates per `TESTING.md`, `SECURITY.md`, and `.github/workflows/ci.yml`.

**Decision state:** M0 is unblocked; accepted decisions are in [`ADR-LOG.md`](docs/adr/ADR-LOG.md), and [D-061](docs/decisions/open-decisions.html#d-061) is the sole open M1 blocker.

**DoD:** the scaffold compiles on Linux, macOS, and Windows; its canonical contracts and fixtures are sufficient to implement M1 without architectural guesses; and all M0 gates defined by the canonical test, security, and CI sources pass.

### M1 — Loop Agent MVP (standalone CLI)

**Wedge:** Loop Agent execution wedge — prove deterministic, reusable, evented agent loops as a deterministic, auditable, reusable agent-loop runtime (not a generic coding agent).

**Status:** M1 implementation is in progress. The standalone CLI runtime, JSONL event stream, local session log, replay/tail/resume commands, fixture registry loading and validation gates are in active hardening against the DoD.

**Deliverables:**

- Standalone CLI Loop Agent (human CLI run path).
- Headless JSONL event stream over stdout.
- Local append-only session/transcript log (ADR-0037); initial resume/tail/replay behavior over the log.
- Public runtime event emission as a stable append-before-publish contract with bounded near-real-time delivery and sequence replay (ADR-0036, ADR-0059).
- Building-block registry for Tools, Instructions, Phases, Loops and Connections using explicit by-name/id references, canonical serialization and cycle detection (ADR-0031).
- Deterministic FSM phase/step engine: phase order, available tools, instruction loading and state transitions are deterministic; LLM/tool outputs are inputs to deterministic transitions.
- Deterministic, cache-stable `loop-context-v0` compilation over mandatory active scope plus narrowly bounded continuity, with reproducible per-turn manifests; persisted compaction and retrieval are post-M1 (ADR-0050, ADR-0058).
- Script-defined Tools/Instructions/Phases/Loops with recursive composition (`Loop` as a building block).
- Event-driven execution: no polling loop for normal agent progress.
- Runtime kernel: deterministic bounded in-process fixture interpretation plus session event and context-manifest logs. External subprocess timeouts, bounded stdout/stderr, per-tool run logs and `tool.timed_out` remain post-M1.
- Deterministic in-process enforcement/emulation for declared command, parameter, read/write, protected-path and deny-all network capabilities per loop. Linux-target policy rejects non-empty network allowlists; Linux Landlock/seccomp OS enforcement and macOS Seatbelt parity are post-M1 targets (ADR-0051, ADR-0052).
- Protocol adapter that emits normalized `proto` v0 events.
- D-015 golden loops and sandbox-negative tests.

**DoD:** a multi-phase local loop with a subloop runs headless from the CLI; compiles deterministic, budget-safe provider context and manifests; appends every canonical event before publishing it; emits the expected JSONL stream; persists/replays/tails/resumes the local session log; enforces phase/tool scoping; writes session event and context-manifest logs; and passes context, FSM, event-ordering, transcript-persistence and sandbox-negative policy-emulation tests (with macOS policy-artifact parity checks). It also meets the `TESTING.md` coverage gate (ADR-0022/ADR-0060) and all Loop Agent M1 budgets in `PERFORMANCE.md`. Loop Agent runs standalone with no dependency on Meta-Harness or Liquid, and no Loop Agent MVP feature depends on a Watershed project-history/VCS engine.

### M2 — Meta-Harness MVP + AgentPulse

**Wedge:** Meta-Harness team/control/governance wedge — turn Loop Agent and external agents into a controllable, observable, measurable system. Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained headless control plane** with CLI/API/service surfaces — usable without Liquid. Liquid integrates later as a client of these surfaces. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI (headless user/admin: run/session/config/metrics commands).
- Local service/daemon shape (sidecar for Liquid / standalone local daemon); remote server is a documented later extension unless a decision pulls it in (deployment modes: D-022).
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, schedule/automation control, AgentPulse queries, approval/reject/revert (transport: D-023).
- Central configuration model that resolves shared Watershed building blocks to the correct agent CLI (Loop Agent, Codex CLI, Claude Code, Pi Agent, etc.).
- Control plane: session registry, routing, task state, attention state and schedule/event triggers; schedule/automation skeleton.
- Executors: local executor first, remote executor as a documented extension point unless explicitly pulled into the MVP by a decision.
- Adapters: Loop Agent (via its public runtime surfaces) + at least one external CLI adapter.
- Event/transcript ingestion from agents; artifact/log/handoff indexing (logs, structured summaries, host-provided diffs, handoff packs, checkpoints).
- AgentPulse v0 metrics: rework ratio, first-attempt success and cost-per-productive-outcome using formulas decided before M2 implementation; computed and stored by Meta-Harness and queryable through CLI/API.
- Policy-gated configuration writes with audit trail and review flow.
- **No rich standalone GUI** (a minimal admin/status UI is out of M2 scope and must not duplicate Liquid; packaging: D-021).

**DoD:** monitor, steer and configure at least two different CLI agents from one control surface, with both represented through one normalized session/event model; Meta-Harness runs without Liquid, and Liquid integration is possible through the public API/protocol; Loop Agent integration uses Loop Agent's public runtime surfaces (not its internals); shared config resolves to agent-specific runtime config without maintaining duplicated per-agent config directories for the same capability; AgentPulse reports decided v0 metrics and is queryable through CLI/API; all sensitive config changes require approval and leave an audit record.

### M3 — Liquid MVP (standalone workspace product)

**Wedge:** Liquid long-term workspace/action wedge — prove safe human/agent co-editing of workspace state with attributed, reviewable, reversible action history. Emphasize user-controlled workspace state and reversible external-agent edits; do not build a generic Notion clone.

M3 delivers Liquid as a **self-contained native workspace/app-building product** that is useful with neither Loop Agent nor Meta-Harness installed; agent integrations are optional. Full product/runtime detail: [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html).

**Deliverables:**

- Native Rust + Dart app shell (after the UI framework decision D-009 closes).
- Local workspace store (D-029).
- Internal action-history / workspace-VCS model (D-028): append-only action log + snapshots/checkpoints; actor/origin attribution; diff; revert (D-031). This is a workspace VCS over Liquid's own data, **not** a project-code VCS.
- Workspace → dashboards → views → components model and connection model (D-033).
- PowerBar (incl. commands that start/steer sessions via Meta-Harness).
- Built-in components: note/document, table, chart, script, file/link/source.
- Liquid CLI for workspace read/edit and action-history commands; local API/service for external agents/tools (D-027). Every UI/CLI/API mutation goes through one permissioned pipeline and records an action; no hidden writes (D-032).
- Local script component sandbox (D-034).
- Liquid AI assistant skeleton, using the same mutation/action-history pipeline.
- Optional Meta-Harness client component and optional Loop Agent transcript/session component. When integrated, Liquid **consumes** Meta-Harness (session dashboard, transcript component, AgentPulse dashboard, config editor, approvals inbox, schedule builder, automation views); it does **not** implement its own session backend, config resolver, scheduler, AgentPulse engine or adapter layer, and Loop Agent/Meta-Harness never mutate Liquid storage directly (boundaries: D-025, D-027).

**DoD:** a user can, **without any agents installed**, create a useful dashboard, add/edit/connect components, run a script component over local data, and use PowerBar for workspace actions; Liquid AI can propose or modify a dashboard/component through the same mutation pipeline; an external agent can read permitted workspace info and propose/apply a permitted mutation through the CLI/API; every mutation is recorded in the action history; a faulty external-agent mutation can be reverted; the workspace can be restored to a previous checkpoint/snapshot. Optional: render Meta-Harness + AgentPulse views in a dashboard and start a loop from the PowerBar.
