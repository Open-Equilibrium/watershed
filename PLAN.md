# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance targets are canonical in `PERFORMANCE.md`.

Created: 2026-06-05

## MVP boundary

The first MVP is **Flow Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. Project-code-history/VCS questions are deferred until after the Flow Agent + Meta-Harness MVPs validate the core workflow. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3).

## Sequencing and adoption

Build the shared substrate, then Flow Agent, Meta-Harness and Liquid. This dependency/order-of-risk sequence validates execution, then multi-agent governance, then the integrated workspace without letting the broadest surface block earlier validation. All three remain independently usable layers of one platform; integration is canonical in `VISION.md`.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Flow Agent and Meta-Harness wedges are validated.

## Milestones

### M0 — Flow Agent MVP implementation packet + walking skeleton

**Wedge:** Flow Agent execution wedge (developer/open-source credibility).

**Purpose:** establish the implementation packet and walking skeleton needed to build the standalone Flow Agent MVP without inventing architecture.

**Deliverables:**

- Rust workspace and the `core`, `proto`, `flow-agent`, `meta-harness`, and `liquid` scaffold.
- Versioned building-block, event, runtime, session, policy, and sandbox contracts. Canonical owners: `PROTOCOL.md`, `SECURITY.md`, and the Flow Agent V-Spec.
- Deterministic fixtures and expected streams per `TESTING.md` (ADR-0034).
- Cross-platform CI, dependency, coverage, link, and render gates per `TESTING.md`, `SECURITY.md`, and `.github/workflows/ci.yml`.

**Decision state:** M0 and M1 are unblocked; accepted decisions are in [`ADR-LOG.md`](docs/adr/ADR-LOG.md).

**DoD:** the scaffold compiles on Linux, macOS, and Windows; its canonical contracts and fixtures are sufficient to implement M1 without architectural guesses; and all M0 gates defined by the canonical test, security, and CI sources pass.

### M1 — Flow Agent deterministic runtime foundation

**Status:** Implementation complete on this branch; pending maintainer review and merge.

**Purpose:** establish the deterministic, auditable runtime contracts that later practical execution and OS isolation extend without presenting fixture behavior as productive provider or process execution.

**Deliverables:**

- Strict Flow registry and policy contracts with canonical serialization and bounded recursive resolution.
- Pure deterministic Flow planning, FSM orchestration and exactly-once apply over the fixture executor.
- Fixture/stub execution only when the workspace explicitly selects the canonical fixture profile; other workspaces fail closed before provider or tool side effects.
- Canonical Flow runtime events, append-only session history, replay, tail and resume.
- Deterministic, cache-stable `flow-context-v0` compilation and reproducible context manifests.
- Deterministic in-process policy enforcement/emulation for declared command, parameter, path, protected-path and deny-all network decisions.
- Cross-platform functional, coverage and Linux performance gates defined by `TESTING.md`, `PERFORMANCE.md` and CI.

M1 does not provide a real provider adapter, general external process execution, a complete POSIX shell, OS-enforced isolation, positive network grants, or public session export/delete/prune operations.

**DoD:** a fixture-profile workspace runs multi-phase Flows and Subflows headlessly; planning is side-effect-free; each planned fixture side effect is applied at most once; non-fixture execution fails closed; canonical events and context manifests are persisted and replayed/tailed/resumed without repeating completed side effects; controlled returns release session locks while preserving valid artifacts; policy-emulation and sandbox-negative tests remain explicit about the absence of an OS boundary; and every M1 test, coverage and performance gate passes. Flow Agent remains standalone and has no Watershed-owned project-code VCS behavior.

### M1.1 — Flow Agent practical execution

**Purpose:** turn the deterministic M1 foundation into a useful local Flow Agent without claiming the OS isolation reserved for M1.2.

**Deliverables:**

1. A provider abstraction with at least one real provider adapter.
2. Typed user inputs.
3. Typed Connection values.
4. Typed Tool and Artifact outputs for `flow-context-v0` or an explicitly versioned successor.
5. Invocation parameters validated against each Tool's `allowed_parameters`.
6. A general bounded external subprocess runner.
7. Predefined commands launched by direct exec without shell parsing, PATH lookup or ambient environment inheritance.
8. Own-script execution through one fixed runner with a bounded runtime and no implicit interpreter selection.
9. Timeouts and cancellation.
10. Bounded stdout and stderr.
11. Per-Tool run logs.
12. Actual `tool.timed_out` and real Tool-failure event emission.
13. The M1 pure-plan/exactly-once-apply boundary for real providers and Tools.
14. Whole-bundle session export, delete, prune and storage/quota status with retention configuration.
15. No Watershed-owned project-code VCS behavior.

**DoD:**

- A non-fixture workspace runs a real Flow through a real provider adapter.
- At least one predefined command and one own-script Tool use the bounded runner.
- Inputs, Connection values and invocation parameters are typed and validated.
- Provider and Tool side effects occur exactly once per planned invocation.
- Timeout, cancellation, output caps and Tool logs are tested.
- Export, delete and prune operate on the complete session bundle.
- No capability depends on Meta-Harness or Liquid.
- Current gates plus the M1.1 performance and security budgets decided before implementation pass.

### M1.2 — Flow Agent OS isolation

**Purpose:** enforce the declared policy as an operating-system boundary around real provider and Tool processes.

**Deliverables:**

1. Linux Landlock filesystem restrictions plus seccomp or an equivalent process/syscall boundary, inherited by child processes.
2. A macOS Seatbelt profile with semantic parity to canonical policy artifacts and child-process inheritance.
3. A Windows enforcement strategy selected through [D-047](docs/decisions/open-decisions.html#d-047); no parity claim before that decision and its evidence.
4. Deny-by-default egress enforcement; CIDR/port grants remain blocked until [D-046](docs/decisions/open-decisions.html#d-046) is decided and proven, including DNS, DoH, DoT and child-process escape tests.
5. An escape matrix covering traversal, symlinks, hardlinks, rename/create races, interpreter escape, environment and credential leakage, child processes, direct and indirect network access, protected paths, process spawning, timeout and cancellation termination.
6. Optional container or microVM hardening that does not replace the OS baseline unless a later accepted decision changes it.

**DoD:**

- Real Tool processes cannot exceed declared read, write, network or process boundaries on every platform for which support is claimed.
- Negative tests exercise the applied OS boundary, not M1 policy emulation.
- The canonical policy artifact and applied policy are demonstrably equivalent.
- Child processes inherit restrictions and fail-open behavior is prevented or remains a release blocker.
- Platform differences and tested coverage are explicit; no security claim exceeds the evidence.

### M2 — Meta-Harness MVP + AgentPulse

**Wedge:** Meta-Harness team/control/governance wedge — turn Flow Agent and external agents into a controllable, observable, measurable system. Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained, host-scoped headless control plane** with CLI/API/service surfaces — usable without Liquid. Each instance controls only CLI agents on its own host. Its public API remains transport-neutral; D-023 selects the local and authenticated remote bindings. Liquid integrates later as a client of one or more instances. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI (headless user/admin: run/session/config/metrics commands).
- Local service/daemon shape (sidecar for Liquid or standalone daemon) with the transport-neutral API and D-023 bindings.
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, agent schedule/trigger control, AgentPulse queries, approval/reject/revert (transport: D-023).
- Central configuration model that resolves shared Watershed building blocks to the correct agent CLI (Flow Agent, Codex CLI, Claude Code, Pi Agent, etc.).
- Control plane: session registry, routing, task state, attention state and agent schedule/trigger skeleton.
- Host-local executor that rejects cross-host agent-process control.
- Adapters: Flow Agent (via its public runtime surfaces) + at least one external CLI adapter.
- Event/transcript ingestion from agents; artifact/log/handoff indexing (logs, structured summaries, host-provided diffs, handoff packs, checkpoints).
- AgentPulse v0 metrics: rework ratio, first-attempt success and cost-per-productive-outcome using formulas decided before M2 implementation; computed and stored by Meta-Harness and queryable through CLI/API.
- Policy-gated configuration writes with audit trail and review flow.
- **No rich standalone GUI**; M2 packaging, including whether it adds a small status screen after the headless controller works, remains D-021.

**DoD:** monitor, steer and configure at least two different CLI agents on the Meta-Harness host from one control surface, with both represented through one normalized session/event model; reject attempts to claim or control a process on another host; run without Liquid; expose the public API/protocol through the bindings selected by D-023; integrate Flow Agent through its public runtime surfaces (not its internals); resolve shared config without duplicated per-agent config directories for the same capability; report decided AgentPulse v0 metrics through CLI/API; and require approval plus an audit record for every sensitive config change.

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
