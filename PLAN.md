# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance targets are canonical in `PERFORMANCE.md`.

Created: 2026-06-05

## MVP boundary

The first MVP is **Flow Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. Project-code-history/VCS questions are deferred until after the Flow Agent + Meta-Harness MVPs validate the core workflow. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3).

## Sequencing and adoption

Build the shared substrate, then Flow Agent, Meta-Harness and Liquid. This dependency/order-of-risk sequence validates execution, then multi-agent governance, then the integrated workspace without letting the broadest surface block earlier validation. All three remain independently usable layers of one platform; integration is canonical in `VISION.md`.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Flow Agent and Meta-Harness wedges are validated. Long-term architecture must remain valid whether inference is local or remote: the provisioned core path is offline-capable, each Flow's model/runtime requirements resolve through a device-local Runtime binding, and cross-device continuation creates Conversation branches instead of transferring live host ownership. Canonical integration detail is in `VISION.md`.

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

**Status:** Complete.

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

**DoD:** a fixture-profile workspace runs multi-phase Flows and Subflows headlessly; planning is side-effect-free; each planned fixture side effect is applied at most once; non-fixture execution fails closed; canonical events and context manifests are persisted and replayed/tailed/resumed without repeating completed side effects; one host-local lease authorizes each active workspace/session pair, direct workspace-marker mutation cannot grant or revoke it, process exit releases it, and controlled returns preserve all operation, writer-finalization, marker-validation and lease-release failures plus valid artifacts; policy-emulation and sandbox-negative tests remain explicit about the absence of an OS boundary; and every M1 test, coverage and performance gate passes. Flow Agent remains standalone and has no Watershed-owned project-code VCS behavior.

### M1.1 — Flow Agent practical execution

**Status:** The final M1.1 Maintainer decisions are implemented; repository closeout is in progress.

#### M1.1 entry criteria — architecture hardening

The M1 deterministic runtime foundation satisfies the following criteria. They remain mandatory boundaries before real provider integration or general subprocess execution begins (ADR-0079):

1. **Protocol validation ownership:** `proto` alone owns Event-envelope and Event-payload structure validation; `flow-agent-core` delegates it. Runtime retains only stream, sequence, budget and lifecycle invariants, without parallel payload schemas or duplicated `EventType` matches.
2. **Real `core-script` modules:** production uses real Rust modules with explicit visibility/imports, no `include!` assembly and a minimal public API.
3. **Explicit Flow Agent runtime boundaries:** internal structure does not depend primarily on flat `pub use module::*` namespaces or systematic `use super::*`; imports, re-exports and security-critical interfaces are targeted.
4. **Responsibility-based module split:** separate planning/apply; Event construction/persistence; Protocol lifecycle validation; Run reservation/locks; Run bundle inventory; Run reader/replay/tail; Resume orchestration; Context compilation; fixture Tool execution; capability-relative filesystem operations; CLI parsing/dispatch; CLI live streaming; and CLI tail. Do not split by line count alone.
5. **Explicit finite execution-plan IR:** `FlowExecutionPlan` is a typed finite IR. Planning has no provider/Tool adapter and cannot constructively perform side effects; invocation IDs are planned; Apply consumes only the plan; Resume distinguishes pending from completed intents; a second full FSM pass is not a substitute for the IR.
6. **Test architecture:** split large tests along the same responsibilities, including Protocol payload versus lifecycle and Run reservation versus bundle, corruption and Resume. Tests remain behavior-focused and the coverage threshold is unchanged.
7. **CLI architecture:** `main.rs` is only the composition root; argument/usage parsing, dispatch, streaming and tail are separate internal modules, without requiring a product-framework decision.
8. **Completion state:** the criteria are complete in M1; provider/subprocess work must preserve them and remains separately scoped to M1.1.

**Purpose:** turn the deterministic M1 foundation into a useful local Flow Agent without claiming the OS isolation reserved for M1.2.

**Deliverables:**

1. A provider abstraction with at least one real provider adapter.
2. Explicit non-interactive `flow init`, `flow validate` and custom `flow create <kind>` authoring for the four registry kinds: Tool, Instruction, Phase and Flow. Creation has complete preflight, no overwrite and no implicit attachment or discovery.
3. Typed selected-root-Flow input and closed typed runtime values.
4. Parameterized Instructions whose declared `{{name}}` placeholders bind typed Phase input values; explicit output contracts for every Phase.
5. Recursive Phases: a leaf runs the provider loop, while a composite runs only its ordered child Phases and selects its declared result child.
6. Whole successful result handoff between selected sibling Phases by default, with ordered inline forward Transitions using exact typed equality.
7. Declarative Phase loops bounded to 1–32 local iterations and 512 Phase iterations per top-level Flow.
8. Phase-scoped Tool availability: the provider may request an available Tool zero or more times; a Tool reference never invokes it automatically.
9. Provider-requested invocation parameters validated against each Tool's `allowed_parameters`, with canonical bounded Tool results for events and Run Logs.
10. A general bounded external subprocess runner.
11. Predefined commands launched by direct exec without shell parsing, PATH lookup or ambient environment inheritance.
12. Own-script execution through one fixed runner with a bounded runtime and no implicit interpreter selection.
13. Timeouts, cancellation and bounded stdout and stderr.
14. Per-Tool Run Log projections plus actual `tool.timed_out` and Tool-failure events.
15. Durable intent around every explicit provider or provider-requested Tool attempt; an uncertain attempt is never relaunched automatically, and `flow reconcile-tool <conversation-id> <run-session-id> --result <file|->` settles exactly one eligible Tool attempt from bounded external evidence.
16. Versioned Conversation trees over linear Runs, including creation, continuation, branching, recovery and paged status, as defined in `PROTOCOL.md`.
17. Explicit `openai-codex` project configuration, browser/device authentication and a protected Flow-owned credential record.
18. Automatic root `AGENTS.md` loading. Manually referenced Instruction source filenames have no authority or naming semantics and may, for example, be named `SYSTEM.md`.
19. No Watershed-owned project-code VCS behavior.

**DoD:**

- A non-fixture workspace runs a real Flow through a real provider adapter.
- Init, Validate and Create for Tool, Instruction, Phase and Flow satisfy the exact long-flag and delimited-group grammar in `PROTOCOL.md` plus the identity-bound transaction contract (ADR-0104); valid output round-trips through registry validation, while failure preserves existing bytes and publishes no partial output.
- Recursive composite/leaf execution, typed Instruction binding, output contracts, whole-result handoff, ordered forward Transitions and both loop bounds are enforced before or during deterministic orchestration.
- At least one predefined command and one own-script Tool use the bounded runner.
- Root input, Phase results, Instruction parameters and provider-requested Tool parameters are typed and validated. Tool declarations make capabilities available but cause no automatic execution.
- Every explicit provider or provider-requested Tool attempt records durable intent; an intent without a committed terminal result is reported as uncertain. Tool reconciliation accepts one canonical bounded result, derives exactly one eligible attempt and never redispatches it.
- Timeout, cancellation, output caps and Tool logs are tested.
- Every Conversation branch remains navigable in one append-only history. Latest-entry continuation and explicit older-entry branching create new Runs from the selected terminal compact snapshot after registry-drift validation, never roll back filesystem or external effects, and retain descendants. Exact two-id recovery consumes the same bounded snapshot model without redispatching durable external results.
- No capability depends on Meta-Harness or Liquid.
- Current gates plus the M1.1 performance and security budgets decided before implementation pass.

**Explicit post-M1.1 Flow Agent work:** later milestones may add typed projections or mappings between Flow, Phase, Instruction, subflow, Tool and addressable artifact inputs/outputs; backward edges or an expression language; general retry/fallback policies; and automatic Tool imports referenced by Instructions. Dynamic proposals to add a Phase, child Phase or Flow at run time—including one-run versus persisted approval and an operator opt-out—also remain deferred. A durable ordered Conversation-status inventory may replace the query-only CV-03 admission bound when larger inventories are required; it must define transaction, recovery, migration and consistency rules without replacing Conversation or migration authority. Any future routing or proposal surface requires a finite schema, permissions, complete allowed/rejected matrix, provenance, replay and approval decision before enablement. The M1.1 routing boundary is canonical in `PROTOCOL.md`.

#### Accepted post-M1.1 architecture corrections

These targets are accepted but not implemented by the current M1.1 code:

- Replace `.flow/config.yaml`, the Workspace registry and automatic root `AGENTS.md` loading with one Global Flow configuration authority. Project-specific behavior is an explicitly authored/selected Flow; no Workspace-local config is discovered or merged. Exact global storage, authoring, fixtures and legacy handling remain [D-057](docs/decisions/open-decisions.html#d-057).
- Add a provider-neutral local-inference path so a provisioned host can run the complete local core path with network interfaces disabled. Portable requirements and device-local Runtime bindings remain separate; provisioning, model/runtime evidence and resource admission remain [D-059](docs/decisions/open-decisions.html#d-059), with the local inference process trust boundary in [D-061](docs/decisions/open-decisions.html#d-061).
- Add a public, versioned Portable continuation contract. A verified terminal checkpoint imported on another device creates a child Run, rebinds destination authority and capabilities, and never replays completed effects. Disconnected continuations branch; exact in-flight recovery remains distinct. Archive, transfer, fencing and tree-navigation details remain [D-058](docs/decisions/open-decisions.html#d-058), archive authenticity in [D-062](docs/decisions/open-decisions.html#d-062), and offline approvals/revocation in [D-060](docs/decisions/open-decisions.html#d-060).

No implementation or release claim follows from the documentation decision alone. Each target requires its red behavior tests, finite security and compatibility contract, implementation and ordinary repository gates in a separately scoped change.

### M1.2 — Flow Agent OS isolation

**Purpose:** replace M1.1's productive direct-Tool path with a Flow-owned Executor boundary that enforces each declared Tool policy at the operating-system boundary while preserving deterministic fixture execution and a working default installation.

**Deliverables:**

1. A versioned one-shot Executor protocol for exactly one Tool invocation: Flow Agent owns policy validation, Executor selection and lifecycle, bounded request/result validation, durable attempt state and fail-closed errors. The companion process receives one JSON request on stdin, returns one JSON result on stdout and is resolved only from an administrator-configured absolute path. No daemon, socket, pool or remote transport is in M1.2.
2. One official Default Sandbox Executor installed by the standard Flow Agent installation path. `--no-default-executor` is an explicit administrator opt-out; it preserves authoring, validation and fixture execution but leaves productive execution fail-closed until a Custom Executor is configured. Flow Agent provides the protocol, implementer documentation, actionable diagnostics and an advisory compatibility probe, but makes no third-party compatibility or security guarantee.
3. Ubuntu 24.04 x64 enforcement through Bubblewrap namespaces/mounts plus seccomp, with inherited descendant confinement and deny-all Tool networking. There is no Landlock-only or unsandboxed fallback.
4. macOS 26 arm64 enforcement through a native Seatbelt profile with semantic parity to the canonical policy and deny-all Tool networking, inherited by descendants.
5. The unchanged deterministic Fixture executor and fake-Executor conformance fixtures, so M1/M1.1 contracts remain testable before, during and after backend implementation.
6. A hostile escape matrix covering traversal, symlinks, hardlinks, rename/create races, interpreter escape, environment and credential leakage, child processes, direct and indirect network access, protected paths, process/session escape, timeout, cancellation and teardown.

**DoD:**

- A standard supported-platform installation passes its readiness self-test and runs a productive Flow out of the box; both standard and opt-out installation paths have automated acceptance tests.
- Real Tool processes cannot exceed declared read, write, deny-all network or process boundaries on every exact platform for which support is claimed. Provider traffic remains Flow Agent traffic outside the Tool Sandbox.
- Protocol conformance tests cover success, unsupported versions and policy, malformed/oversized output, timeout, premature exit, missing evidence and unavailable configuration without spawning a Tool after failed preflight.
- Negative tests exercise the official applied OS boundary, not M1 policy emulation, and demonstrate equivalence between canonical policy and applied restrictions.
- Descendants inherit restrictions; backend, protocol or readiness failure never falls back to M1.1 direct execution; platform differences and tested coverage are explicit.
- The architecture and plain-language comparison with Pi Coding Agent and Codex CLI remain visible in [`docs/concept/flow-agent-executor-architecture.md`](docs/concept/flow-agent-executor-architecture.md).

**Explicit post-M1.2 work:** Windows productive execution remains disabled until [D-047](docs/decisions/open-decisions.html#d-047) selects and proves a boundary. Positive CIDR/port grants remain disabled until [D-046](docs/decisions/open-decisions.html#d-046) is decided and proven. OCI/Docker/Podman, Lima/Apple Container, Firecracker/Cloud Hypervisor, gVisor, Kata Containers and Gondolin are non-binding future integration candidates, not support promises. A later milestone may add an independently reviewed backend or Custom Executor without transferring Executor ownership to Meta-Harness or Liquid.

### M2 — Meta-Harness MVP + AgentPulse

**Wedge:** Meta-Harness team/control/governance wedge — turn Flow Agent and external agents into a controllable, observable, measurable system. Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained, host-scoped headless control plane** with CLI/API/service surfaces — usable without Liquid. Each instance controls only CLI agents on its own host. Its public API remains transport-neutral; D-023 selects the local and authenticated remote bindings. Liquid integrates later as a client of one or more instances. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI (headless user/admin: run/session/config/metrics commands).
- Local service/daemon shape (sidecar for Liquid or standalone daemon) with the transport-neutral API and D-023 bindings.
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, agent schedule/trigger control, AgentPulse queries, approval/reject/revert (transport: D-023).
- Central configuration model that resolves shared Watershed building blocks to each target's documented config surface. Flow Agent receives only its global authority and explicit Flows rather than a project-local config layer; device-local Runtime bindings remain owned by the execution host.
- Control plane: session registry, routing, task state, attention state and agent schedule/trigger skeleton.
- Host-local Meta-Harness agent executor that rejects cross-host agent-process control and never manages a Flow Executor or Tool Sandbox (backend integration: [D-048](docs/decisions/open-decisions.html#d-048)).
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
- Optional Meta-Harness projections show friendly harness/adapter-configuration/location/session choices and separate config management while preserving instance identity, freshness and authority. Flow Agent selection requires an explicit Flow from its global authority; other adapters retain their documented selection model. Live commands route to the owning instance; a future portable checkpoint continued elsewhere appears as a new child Run/branch, not the same live process.

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
