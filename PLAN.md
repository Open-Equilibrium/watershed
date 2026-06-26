# Plan

Implementation plan as milestones with deliverables and a Definition of Done (DoD). Performance targets are **not** repeated here — see `PERFORMANCE.md`. Dates are targets the maintainer fills in; the Progress Log is timestamped on completion.

Created: 2026-06-05
Updated: 2026-06-24

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
  - list of sandbox-escape tests to implement in M1;
  - network deny-by-default policy model;
  - declared read/write roots model;
  - timeout/headless execution model.
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

D-008 and D-046 are closed for M1 in ADR-0050/ADR-0051: M1 context handling is deterministic rule/window selection only, and M1 Linux network enforcement is fail-closed deny-all with non-empty allowlists rejected in OS-enforced runs. D-019 (RPC command/request shape) and D-020 (embedded core API scope) remain post-M1 seams and do not block M1.

**DoD / pass-fail definition:**

- Pass if a fresh Codex session can read `README.md`, `AGENTS.md`, `PLAN.md`, `PROTOCOL.md`, `SECURITY.md`, `TESTING.md`, the Loop Agent V-Spec and the M0 ADR entries in `docs/adr/ADR-LOG.md`, then create the M1 implementation PR without stopping for architecture questions.
- Pass if the repo contains the M0 scaffold, placeholder crates compile, CI runs green on Linux + macOS across the mandatory M0 gates (`cargo fmt --check`, `cargo clippy`, `cargo nextest run`, `cargo audit`/`cargo deny`, `lychee` docs link-check + `pnpm run docs:render-check`), and the D-015 fixture suite follows the contract in `TESTING.md` and the M0 scaffold includes checked-in expected event streams.
- Fail if Codex must choose protocol transport, script schema, CLI shape, sandbox depth, crate layout, D-015 fixture strategy, fixture discovery/stub-model activation, predefined-command registry trust boundary, coverage or invocation contract.

### M1 — Loop Agent MVP (standalone CLI)

**Wedge:** Loop Agent execution wedge — prove deterministic, reusable, evented agent loops as a deterministic, auditable, reusable agent-loop runtime (not a generic coding agent).

**Deliverables:**

- Standalone CLI Loop Agent (human CLI run path).
- Headless JSONL event stream over stdout.
- Local append-only session/transcript log (ADR-0037); initial resume/tail/replay behavior over the log.
- Public runtime event emission as a stable contract (ADR-0036).
- Building-block registry for Tools, Instructions, Phases, Loops and Connections using explicit by-name/id references, canonical serialization and cycle detection (ADR-0031).
- Deterministic FSM phase/step engine: phase order, available tools, instruction loading and state transitions are deterministic; LLM/tool outputs are inputs to deterministic transitions.
- Deterministic M1 context window selection over explicit inputs, instructions, transcript prefix and fixture data only; embeddings, RAG and adaptive compaction are post-M1 (ADR-0050).
- Script-defined Tools/Instructions/Phases/Loops with recursive composition (`Loop` as a building block).
- Event-driven execution: no polling loop for normal agent progress.
- Runtime kernel: bounded/headless tool runs, timeouts, structured stdout/stderr, `.loop/logs` or equivalent run logs.
- Linux policy→sandbox enforcement for declared command, parameter, read/write, protected-path and deny-all network capabilities per loop. OS-enforced runs reject non-empty network allowlists; macOS validates the policy artifact and sandbox-negative expectations until Seatbelt parity lands (ADR-0051).
- Protocol adapter that emits normalized `proto` v0 events.
- D-015 golden loops and sandbox-negative tests.

**DoD:** a multi-phase local loop with a subloop runs headless from the CLI, emits the expected JSONL event stream, persists it to the local session log, enforces phase/tool scoping, writes structured logs, passes deterministic FSM tests, event-ordering and transcript-persistence tests, and Linux sandbox-escape tests (with macOS policy-artifact parity checks), meets the ≥95% line-coverage gate (`cargo llvm-cov`, ADR-0022) and the Loop Agent M1 performance budgets in `PERFORMANCE.md` (ADR-0049). Loop Agent runs standalone with no dependency on Meta-Harness or Liquid, and no Loop Agent MVP feature depends on a Watershed project-history/VCS engine.

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

## Ordered follow-up steps to start M1 with Codex

1. Finish maintainer doc polish, then move/push the project to the official `Open-Equilibrium/watershed` repo.
2. Verify official `main` has green CI and GitHub branch protection/ruleset before starting PR-only work.
3. Start the first M1 topic branch from official `main`: executable `loop run smoke-loop --emit jsonl` over the checked-in fixture with the stub model.
4. Add runtime gates as implementation lands: ≥95% coverage, D-015 golden diffs, M1 performance budgets, Linux sandbox-negative enforcement and macOS policy-artifact parity.
5. Expand in order: `hello-loop`, session log replay/tail/resume, then full sandbox policy enforcement.
6. Start Meta-Harness M2 planning only after Loop Agent M1 is green and standalone.

## Progress Log (timestamped)

- `2026-06-24` — M1 preflight decisions closed: Loop Agent M1 performance budgets (ADR-0049), deterministic M1 context scope (ADR-0050), fail-closed Linux network enforcement with non-empty allowlists rejected (ADR-0051). Decision docs cleaned up: live open decisions stay in `open-decisions.html`, accepted decisions stay compact in `ADR-LOG.md`.
- `2026-06-23` — M0 scaffold started: Rust workspace/toolchain policy, `proto`, `core-script`, `core-policy`, `loop-agent-core`, `loop-agent-cli`, deterministic D-015 fixture streams, M0 policy expected-output fixtures and GitHub CI gate wiring added. Loop Agent runtime execution remains M1 scope.
- `2026-06-05` — Repo governance & spec set created; name "Watershed" selected (crates.io free).
- `2026-06-09` — MVP boundary clarified: Loop Agent is CLI-only and does not own project-history/VCS behavior in the MVP.
- `2026-06-09` — Meta-Harness scoped as a self-contained headless control plane (CLI/API/service) that runs without Liquid; Liquid is the primary rich UI consuming it. M2 reframed accordingly; D-021…D-025 opened.
- `2026-06-09` — Liquid scoped as a standalone native workspace/app-building product (useful without Loop Agent or Meta-Harness), with its own workspace data, workspace action history/VCS, CLI/API for external agents and a single permissioned mutation pipeline. M3 reframed accordingly; D-026…D-035 opened; D-001 made an umbrella pointer.
- `2026-06-10` — CLI binary names decided: `loop`/`meta`/`liq` (ADR-0013). Liquid NFRs retargeted to tiered, falsifiable budgets (ADR-0014). Contributions switched to DCO-only, no CLA; `CLA.md` removed, D-004/D-011 closed (ADR-0015). SPDX clarified to `AGPL-3.0-only`. OSS hygiene added: project status in README, security reporting contact, Code of Conduct, issue/PR templates, changelog.
- `2026-06-10` — Codex enablement: project config `.codex/config.toml` and repo skill `.agents/skills/autoreview` (vendored, MIT, openclaw/agent-skills) added; AGENTS.md gained a Codex setup section.
- `2026-06-11` — Positioning/licensing decisions closed: platform framing, wedge order, layer positioning, AGPL posture (ADR-0016…ADR-0019; D-036…D-041 closed). GitHub repo `Open-Equilibrium/watershed` created; crates.io name verified free (D-003 note). Codex `tdd` and `commit` skills added; goal-mode stop rule added to AGENTS.md.
- `2026-06-11` — Naming closed (ADR-0020; D-003): official repo `Open-Equilibrium/watershed`; crates.io + pub.dev free; npm not a publishing target. Agentic loop closed: clawpatch PR gate (pinned dev dependency + skill), commit-skill closeout order tests → autoreview → clawpatch → PR, Codex subagents `docs_scout`/`validator`. Hooks evaluated and deferred (experimental, disabled on Windows).
- `2026-06-16` — M0 quality-gate toolchain made mandatory (ADR-0021): `cargo fmt --check`, `cargo clippy`, `cargo nextest run` (deterministic test runner), `cargo audit` + `cargo deny` (dependency hygiene; replaces the prior `cargo vet` mention), `lychee` docs link gate and HTML render checks. CI packet, `TESTING.md`, `SECURITY.md` and the `validator` subagent updated; D-006's CI-gates item was decided in part here; D-006 is fully closed by ADR-0030.
- `2026-06-16` — Codex setup hardened: ≥95% line-coverage gate from M1 (ADR-0022, `cargo llvm-cov`); subagent topology expanded — `validator`→`pr_validator`, added `autoreview_runner`/`clawpatch_runner` (edit) and `doc_sync` (read-only), per-agent gpt-5.5 model + xhigh/medium effort, summaries-with-evidence (ADR-0023); opt-in Codex hooks via `[features] hooks` + `.codex/hooks.json` (PreToolUse Bash guard, Stop closeout check; experimental, Linux/macOS-only, ADR-0024). `ui_validator` (Playwright) deferred to M3.
- `2026-06-16` — Branching model adopted then simplified (ADR-0025): single-tier session→`main` (protected `main`, per-session `<type>/<scope>` branches off `main`); `commit` skill renamed to `git`; subagents never branch (commit only). SessionStart hook dropped as redundant (Codex auto-loads `AGENTS.md`); `[windows]` config note generalized to Win11 + Pop!\_OS + macOS. GitHub branch protection pending the CI workflow (required checks must exist first).
- `2026-06-16` — Orchestration: added `repo_mapper` (gpt-5.4-mini, read-only) for session-start orientation and moved `docs_scout` to gpt-5.4-mini; wired explicit delegation points (start: repo_mapper+docs_scout; tdd: docs_scout; closeout: pr_validator→autoreview_runner→clawpatch_runner→doc_sync) into AGENTS + the tdd skill (ADR-0026).
- `2026-06-16` — Subagent standards compliance made explicit (ADR-0027): each agent's `developer_instructions` reference AGENTS.md (read-on-demand, not duplicated); AGENTS.md declares the binding; closeout chain (`autoreview`/`clawpatch`/`doc_sync`) enforces. Robust to Codex not documenting subagent AGENTS.md inheritance.
- `2026-06-18` — Maintainer concept review: recorded ADR-0028 (Loop Agent build strategy & external-agent reuse — build orchestration in-house, reuse plumbing via general-purpose crates, Codex CLI as a Meta-Harness adapter alongside Claude Code/Pi Agent; no external-executor phase type), recorded as D-042. Added the `PROTOCOL.md` no-co-location invariant. Opened D-043 (deployment topologies / local-cloud placement), D-044 (Loop Agent session durability/resume/ownership lease) and D-045 (Liquid agent-authored custom-UI scope); enriched D-034 (script-runtime trade-off triangle). Sharpened Loop Agent determinism framing (orchestration-not-output; four guarantees) in VISION + Loop Agent V-Spec; clarified Liquid positioning (long-tail/sovereign-workspace moat) and added an audience-convergence note. Drift fixes: Liquid V-Spec §12 perf realigned to ADR-0014 tiered budgets; CHANGELOG ADR range corrected.
- `2026-06-19` — M0 blockers D-002, D-006 and D-012…D-018 closed in ADR-0029…ADR-0037: local stdio JSON-RPC transport with transport-agnostic envelopes, accepted M0 pass/fail checklist, strict YAML 1.2 script format with canonical serialization, M0 sandbox artifacts/tests with M1 Linux enforcement, fixed crate layout, D-015 fixture suite, M1 human CLI + machine-readable stream, v0 event names and append-only `.loop/sessions/<session_id>.jsonl` session store with lowercase path-safe IDs. D-046 opened for positive Linux CIDR egress allow enforcement; superseded by ADR-0051 for M1.
- `2026-06-19` — M0 decision packet branch review found three remaining M0 blockers after D-002, D-006 and D-012…D-018 were recorded: fixture registry discovery/stub-model activation (D-047), trusted predefined-command registry contract (D-048) and the HTML render gate requirement (D-049). These were decided in ADR-0041…ADR-0043; D-050 was later decided in ADR-0045 for exact render command packaging and viewport constants.
