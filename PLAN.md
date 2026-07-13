# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance targets are canonical in `PERFORMANCE.md`.

Created: 2026-06-05
Updated: 2026-07-13

## MVP boundary

The first MVP is **Loop Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. Project-code-history/VCS questions are deferred until after the Loop Agent + Meta-Harness MVPs validate the core workflow. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3).

## Sequencing rationale

Build the shared substrate first, then the most differentiated/validatable layer (Loop Agent) as a standalone CLI, then the layer that depends on it (Meta-Harness), then the integrating surface (Liquid). This keeps the broadest surface (Liquid) from blocking validation while still designing all MVP pieces so they remain compatible with the overall `VISION.md` integration model.

## Platform wedge sequencing

Watershed is one AGPL/free-software platform with three independently usable layers. The milestones validate the layers in dependency/order-of-risk sequence, not as three unrelated products.

1. Loop Agent proves structured agent execution and creates the developer/open-source credibility wedge.
2. Meta-Harness proves multi-agent control, observability, policy, metrics, and creates the team/control/governance wedge.
3. Liquid proves safe human/agent workspace co-editing with reversible history and creates the long-term workspace/action wedge.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Loop Agent and Meta-Harness wedges are validated.

## Milestones

### M0 — Loop Agent MVP implementation packet + walking skeleton

**Wedge:** Loop Agent execution wedge (developer/open-source credibility).

**Purpose:** make the repository ready for a Codex session to implement the Loop Agent MVP without making architectural guesses. Establish the event schema, local runtime surfaces, transcript/session log, script parser, FSM and sandbox policy model that prove deterministic, reusable, evented agent loops. Do not overbuild, and do not scope Loop Agent as a generic coding agent.

**Deliverables:**

- Repo scaffold: Rust workspace, root toolchain policy, `core/`, `proto/`, `loop-agent/`, `meta-harness/`, `liquid/` placeholder crates/packages as needed.
- `core` v0 contracts:
  - building-block/script model types;
  - script parser contract and fixtures;
  - policy model and policy→sandbox compiler contract;
  - identity/permissions placeholder types where needed by the protocol.
- `proto` v0 contract:
  - event envelope fields;
  - session lifecycle messages;
  - loop/activity messages;
  - artifact/log messages;
  - attention messages;
  - generic `error` event family and versioning rules.
- Loop Agent MVP packet (Loop Agent is a **standalone CLI product**; Meta-Harness and Liquid are optional consumers, not prerequisites — see `docs/concept/V-Spec_LoopAgent.html`):
  - CLI command names and flags, including by-name `loop run <name> --emit jsonl`,
    `loop chat` and in-session `/hello-loop`;
  - the M1 runtime surfaces: human CLI and headless machine-readable event stream;
    remote-control/RPC and `loop-agent-core` embedding are designed-for seams, not
    M1 implementation scope;
  - event schema v0 (envelope + runtime event families);
  - local session/transcript store path, retention assumptions and local
    replay/tail/resume semantics;
  - `loop-agent-core` vs `loop-agent-cli` crate boundaries;
  - deterministic FSM model;
  - minimum v0 building-block schema fields and recursion rules (`Loop` is a
    building block);
  - instruction/tool/phase/connection terminology;
  - D-015 fixture suite descriptions and golden-stream contract (see
    `TESTING.md`):
    `smoke-loop`, coverage-driven `hello-loop` and sandbox-negative fixtures,
    all deterministic through a stub model;
  - explicit statement that Loop Agent does not manage VCS in the MVP and that the
    local session store is runtime state, not project history;
  - pass/fail definition such that Codex does not have to invent these surfaces.
- Security packet:
  - exact M0 sandbox output artifact shape per `SECURITY.md`;
  - list of sandbox-negative policy-emulation tests to implement in M1;
  - network deny-by-default policy model;
  - declared read/write roots model;
  - headless in-process M1 boundary and post-M1 subprocess-timeout model.
- CI packet:
  - Linux + macOS workflow plan;
  - `cargo fmt --check`, `cargo clippy` and `cargo nextest run` (deterministic,
    process-isolated test runs) as the M0 lint/test gates;
  - dependency-hygiene gate — `cargo audit` (RustSec advisories) and `cargo deny`
    (license/bans/sources/advisory policy via `deny.toml`); see `SECURITY.md`;
  - docs link/HTML validation gate via `lychee` (link integrity) + HTML render check;
  - coverage harness `cargo llvm-cov nextest` wired now; the ≥95% line-coverage
    gate is enforced from M1 (ADR-0022);
  - M0 pass/fail checklist.
    These gates (`cargo fmt --check`, `cargo clippy`, `cargo nextest`,
    `cargo audit`/`cargo deny`, `lychee` and HTML render validation) are mandatory
    M0 essentials (ADR-0021); D-049/ADR-0043 decides the HTML render requirement,
    D-050/ADR-0045 pins the exact command and viewport constants, and the ≥95%
    coverage gate (`cargo llvm-cov`) is mandatory from M1 (ADR-0022).

**M0-blocking decisions:** none remain. D-002, D-006, D-012…D-018 and D-047…D-050 are decided in ADR-0029…ADR-0037 and ADR-0041…ADR-0045.

D-008/D-057 are closed for M1 in ADR-0050/ADR-0058: M1 provider context uses the single deterministic, cache-stable `loop-context-v0` profile; compaction and retrieval remain post-M1. D-058 is closed by ADR-0059: canonical events append through one serial session writer before bounded near-real-time publication. ADR-0051/ADR-0052 close the M1 network/sandbox behavior: M1 Linux-target network policy is fail-closed deny-all with non-empty allowlists rejected for deterministic in-process runs, while D-046 remains open for post-M1 positive CIDR egress enforcement. D-019 is closed by ADR-0055; D-020 remains a non-blocking post-M1 embedded-API seam. D-056 is closed by ADR-0056: main-branch protection requires the M1 gates for PR merges, while `feat/**` push CI stays advisory. No open decision blocks M1.

**DoD / pass-fail definition:**

- Pass if a fresh Codex session can read `README.md`, `AGENTS.md`, `PLAN.md`, `PROTOCOL.md`, `SECURITY.md`, `TESTING.md`, the Loop Agent V-Spec and the M0 ADR entries in `docs/adr/ADR-LOG.md`, then create the M1 implementation PR without stopping for architecture questions.
- Pass if the repo contains the M0 scaffold, placeholder crates compile, CI runs green on Linux + macOS across the mandatory M0 gates (`cargo fmt --check`, `cargo clippy`, `cargo nextest run`, `cargo audit`/`cargo deny`, `lychee` docs link-check + `pnpm run docs:render-check`), and the D-015 fixture suite follows the contract in `TESTING.md` and the M0 scaffold includes checked-in expected event streams.
- Fail if Codex must choose protocol transport, script schema, CLI shape, sandbox depth, crate layout, D-015 fixture strategy, fixture discovery/stub-model activation, predefined-command registry trust boundary, coverage or invocation contract.

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

**DoD:** a multi-phase local loop with a subloop runs headless from the CLI; compiles deterministic, budget-safe provider context and manifests; appends every canonical event before publishing it; emits the expected JSONL stream; persists/replays/tails/resumes the local session log; enforces phase/tool scoping; writes session event and context-manifest logs; and passes context, FSM, event-ordering, transcript-persistence and sandbox-negative policy-emulation tests (with macOS policy-artifact parity checks). It also meets the ≥95% line-coverage gate (`cargo llvm-cov`, ADR-0022) and all Loop Agent M1 budgets in `PERFORMANCE.md`. Loop Agent runs standalone with no dependency on Meta-Harness or Liquid, and no Loop Agent MVP feature depends on a Watershed project-history/VCS engine.

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
