# Glossary

Canonical terms. Use these exactly; do not introduce synonyms. Tool names are final.

## Platform & strategy

- **Platform** — Watershed as a whole: one AGPL/free-software AI-native work platform for reusable, measurable and reversible agent workflows, composed of three independently usable layers.
- **Platform layer** — One independently usable part of the Watershed platform. There are three: execution, control and workspace/action.
- **Execution layer** — Flow Agent's role: running repeatable, auditable agent workflows.
- **Control layer** — Meta-Harness's role: controlling, observing, measuring and governing many agents.
- **Workspace/action layer** — Liquid's role: human/agent workspace co-editing with reversible action history.
- **Agent workflow** — A structured unit of AI-agent work that can be run, observed, measured and improved (in Flow Agent, realized as a Flow).
- **Reversible agent action** — An attributed workspace mutation that can be inspected and reverted (see Action, Workspace action history).
- **Wedge** — The first narrow adoption path used to validate and grow the platform. Flow Agent is the developer/open-source execution wedge, Meta-Harness the team/control/governance wedge, Liquid the long-term workspace/action wedge.
- **AGPL/free-software posture** — The project's commitment to transparent, inspectable, self-hostable software under the repository's `AGPL-3.0-only` license; not a proprietary/open-core monetization stance.
- **Topic branch** — Short-lived Git branch for one logical change; can span multiple sessions and is PR'd back to `main` (ADR-0046).
- **Git upstream** — Remote-tracking branch a local branch uses by default for pull/push; topic branches must not use `origin/main` (ADR-0048).
- **Commit metadata** — Commit subject, body, comment text and trailers; metadata changes must not change file content (ADR-0048).
- **Unpublished commit** — Commit not yet pushed or otherwise shared; only these commits may be reworded for metadata corrections (ADR-0048).

## Tools

- **Liquid** — Standalone local-first workspace and app-building product built from Pages, Blocks, Views, Sources, Connections and Automations. Every client works through a local replica; optional sync, agent integration and headless execution extend rather than replace that product. CLI binary: `liq` (ADR-0013).
- **Flow Agent** — Host-local, CLI-only, Rust-core, script-driven deterministic Flow runtime (not a generic coding agent). M1 is fixture-bounded; M1.1 adds practical provider/process execution and M1.2 adds OS isolation. CLI binary: `flow` (ADR-0013).
- **Meta-Harness** — Self-contained host-scoped headless control plane over CLI agents on one host (session registry, central config resolution, agent schedules/triggers, artifact indexing, AgentPulse), reachable through local or authenticated remote clients. Runs without Liquid; Liquid is its primary rich UI. CLI binary: `meta` (ADR-0013).
- **Execution location** — User-editable label for the host and owning Meta-Harness target used to start or control a session. Technical identifiers remain internal while routing authority is preserved.
- **Pi Agent** — The Pi CLI agent integration target. Use the full term "Pi Agent" in docs; avoid bare "Pi" except when quoting an external CLI/product name.

## Roles & layers

- **Core** — Shared libraries used by all tools: building-block/script format, identity/permissions, policy→sandbox compiler and config/protocol helpers.
- **Protocol** — The versioned contract over which the tools communicate (the integration seam).
- **Meta-Agent** — The agent that _operates_ the Meta-Harness; either Liquid-native or BYOA. May reconfigure underlying agents under policy + audit control.
- **BYOA** — "Bring Your Own Agent"; plugging an external agent in as the Meta-Agent.
- **AgentPulse** — Meta-Harness subsystem measuring rework ratio, first-attempt success rate, and cost-per-productive-outcome. Meta-Harness computes/stores the metrics; Liquid only renders them.
- **Adapter** — A Meta-Harness subsystem that translates an external agent (Codex CLI, Claude Code, Pi Agent, etc.) into normalized protocol events/commands. Native agent shapes do not leak past the adapter.

## Flow Agent primitives

- **Building Block** — The flexible, modular and recursive unit of configuration; every Tool, Instruction, Phase and Flow is a building block. Flows can contain flows.
- **Building-block registry** — The resolver for addressable Tools, Instructions, Phases, Flows and Connections. In v0 it safely catalogs one-block YAML entries under a configured root, then retains and resolves only the selected Flow's transitive definition closure, once per unique definition.
- **Canonical serialization** — Deterministic UTF-8 JSON of the parsed, semantically validated and registry-resolved selected definition closure; equivalent closures serialize to the same bytes for review, audit and golden tests.
- **Tool** — A capability with an exact command identity (predefined or own script) and declared parameter, path and network boundaries. Nothing outside the declared command is permitted.
- **Policy artifact** — The canonical JSON output produced by `core-policy` to show Tool-scoped capabilities for a target enforcement backend, including deterministic object-key and array ordering. M1 evaluates compatible decisions in process but provides no OS security boundary.
- **Predefined-command registry** — The trusted id-to-executable map used by predefined-command Tools. A script names a `command_id`; Flow Agent resolves it to one executable identity and combines it with the script's literal base `argv` without PATH lookup or shell parsing.
- **Predefined-command Tool** — A Tool that names a fixed command id resolved through the predefined-command registry. M1 fixtures emulate its result; M1.1 direct-execs the resolved command under bounded runtime controls.
- **Allowed parameter** — A reviewed parameter spec for a Tool: exact name, typed value shape, required flag and type-specific constraints such as enum values, string pattern/length or integer range. Unknown parameters and values that fail validation are denied before tool launch.
- **Own-script Tool** — A Tool whose reviewed inline `script_body` uses the fixed v0 `posix-sh` contract. M1 interprets only its bounded fixture subset; real runner and isolation stages are defined in the [Flow Agent V-Spec](docs/concept/V-Spec_FlowAgent.html) and [`SECURITY.md`](SECURITY.md).
- **Instruction** — A modular prompt primitive (`id`, `name`, `prompt`). Carries no phase binding or tool ownership; phases reference instructions and tools.
- **Connection** — A declared relation between building blocks, data sources, events or outputs. Connections make data/control flow explicit instead of hiding it in agent-specific terminology.
- **Phase** — A workflow stage; declares the tools and instructions available within it and contains ordered steps. Authored as a script; a visual graph is a view over that script.
- **Flow** — A fully configured, deterministic AI-native process: a state machine composed of phases and building blocks (1…n agents). A flow is itself a building block.
- **Subflow** — A flow used inside another flow.
- **Flow execution plan** — The finite typed, deterministic and side-effect-free description of resolved Flow invocations, lifecycle boundaries, context inputs and Tool intents consumed by runtime apply.
- **Fixture executor** — The M1 in-process executor enabled only by the canonical fixture profile; it returns deterministic model/Tool doubles and is not a provider, subprocess runner or POSIX shell.
- **Policy emulation** — M1's deterministic in-process evaluation of modeled policy decisions. It proves runtime correctness contracts, not OS isolation.
- **OS isolation** — M1.2 enforcement that prevents real provider/Tool processes and their children from exceeding applied filesystem, process and network boundaries.

## Flow Agent runtime surfaces

- **Session** — One Flow Agent run, identified by a lowercase path-safe `session_id` token per `PROTOCOL.md`; the unit that is started, resumed, tailed and persisted.
- **Transcript** — The ordered record of a session's messages and runtime events; part of durable session history and reconstructable by replay.
- **Durable session history** — The complete append-only session event history plus referenced source artifacts; its events are authoritative for replay, while the full history is authoritative for resume, audit, debugging and future retrieval. Provider-context optimization never deletes either.
- **Session bundle** — All session-owned event and context-manifest segments, immutable hash-addressed objects and definition metadata; export and deletion treat them as one unit.
- **Host-local session ownership lease** — The exclusive operating-system file lease keyed by canonical workspace and `session_id` that alone authorizes one active Flow Agent session on a host. Process exit releases it; it is neither durable nor transferable across hosts.
- **Session marker** — The persistent `.flow/sessions/<session_id>.lock` observability leaf. It is not ownership authority, and creating, deleting or replacing it cannot grant or revoke the host-local session ownership lease.
- **Reservation orphan** — An empty session artifact retained and inventoried when rollback cannot prove identity-bound deletion. Its cleanup failure remains visible, and compare-then-unlink never removes it.
- **Event segment** — One append-only canonical JSONL file within a session bundle; segment rotation bounds individual I/O without starting a new session or resetting sequence/budgets.
- **Resolved flow state** — The current flow invocation, phase, step, active instructions/tools, connections and runtime state, plus values exposed by defined runtime interfaces.
- **Provider context** — The deterministic, bounded projection compiled from resolved flow state and narrowly selected durable history for one model turn; not the full transcript.
- **Context profile** — A versioned deterministic contract for provider-context ordering, budgeting, tokenization/estimation, projections, hashing and cache boundaries; M1 exposes only `flow-context-v0`.
- **Context manifest** — The reproducible per-provider-turn record of the context profile, budget inputs, ordered included/projected sources, omissions, cache boundaries, token estimate and final context hash.
- **Runtime event** — One normalized event Flow Agent emits over its public contract (see `PROTOCOL.md`); the same events feed JSONL mode, future RPC mode and the session store.
- **JSONL event stream** — Flow Agent's headless mode that streams newline-delimited JSON runtime events to stdout for automation/CI/consumers.
- **RPC mode** — Flow Agent's designed-for bidirectional stdin/stdout control mode. ADR-0055 selects the initial command/request shape; runtime events remain the public event contract.
- **Session store** — Flow Agent's local append-only transcript persistence (e.g. `.flow/sessions/<session_id>.jsonl`). Runtime state only; **not** a project VCS/history engine.
- **Live-event notification** — A bounded, best-effort wake-up reporting a session's earliest pending and highest committed sequences; it carries no event payload, and receivers replay events after their cursor from the authoritative session log.
- **Persistence-before-notification** — The local guarantee that one serial session writer successfully appends a canonical event to the authoritative log before updating its high-watermark and attempting a live-event notification; physical `fsync` follows separate bounded durability checkpoints.
- **Flow registry** — The name/id index used by `flow run <name>` and interactive slash commands such as `/hello-flow` inside `flow chat` to resolve a flow definition without requiring a path.
- **Fixture workspace** — A checked-in test workspace whose explicit canonical fixture profile selects the fixture executor and deterministic stub model.
- **Flow definition ID** — The registry/building-block id of a Flow definition; carried in event payloads as `flow_definition_id`.
- **Runtime flow invocation ID** — The `flow_id` assigned to one root-flow or subflow invocation in a session; distinct from the Flow definition ID and linked to a parent by `parent_flow_id`.
- **Stub model** — A deterministic model double used by tests so golden event streams are byte-stable in CI.
- **Golden flow** — A checked-in flow fixture with a deterministic expected event stream used for capture-and-diff validation.
- **Golden event stream** — A checked-in JSONL event stream with fixed fixture IDs, timestamps, sequence values and canonical event JSONL bytes per `PROTOCOL.md`.
- **Environment allowlist** — A tool-scoped policy-artifact field; its M1 availability and restrictions are defined in [`SECURITY.md`](SECURITY.md).
- **Network allow entry** — A typed CIDR/IP egress rule with transport and port; the only v0 way to declare network access. M1 Linux-target deterministic policy rejects non-empty allowlists until D-046 selects a positive egress backend.
- **Protected path** — A path pattern denied by default even inside a declared root unless a flow explicitly grants it.
- **Protected-path grant** — A tool-scoped exception that removes the protected-path deny only when the path is still inside that tool's declared read/write scope.
- **Smoke-flow** — The smallest golden flow: one phase, one tool and one instruction, used as the first localizable gate.
- **Hello-flow** — The showcase golden flow that exercises phases, scoped instructions/tools, connections and subflow reuse.
- **Sandbox-negative fixture** — A tiny flow that intentionally attempts a forbidden operation and must be rejected.
- **HTML render gate** — The CI validation that browser-renders self-contained HTML docs; `pnpm run docs:render-check` invokes `scripts/check-html-render.mjs` at `1440x900` and `390x844` (ADR-0043, ADR-0045).

## Liquid primitives

- **Workspace** — Top-level owner of Pages, Blocks, Sources, Connections, Automations, Roles, History and settings.
- **Page** — Liquid's authored surface: an ordered composition of Block Views with responsive layout metadata. A Page starts as an empty flow and may later be arranged on a grid.
- **Block** — A canonical, typed and addressable unit of content or functionality, such as text, database, code, formula, media, whiteboard, App or agent session. A text toggle is formatting inside a Text Block, not a Block Type.
- **Block Type** — The versioned behavior, state contract, supported Views, ports/actions, permissions and responsive capabilities shared by Blocks of one kind.
- **View** — One placement or representation of a Block. Several Views may expose the same Block on different Pages and stay synchronized because they share canonical Block state. Duplicating creates a new Block; adding a View does not.
- **Connection** — An explicit typed and permissioned data/action relation among Blocks and Sources. It addresses stable objects and ports independently of active View or layout; visual proximity never grants access.
- **Automation** — A Workspace-owned event/condition/action rule over Liquid objects and permitted external actions. It is distinct from Meta-Harness agent scheduling and orchestration.
- **Source** — An external or shared resource addressable from a Workspace. Sources appear beside Pages in the Explorer and use the same open/navigation flow, but are not authored Pages internally.
- **Explorer** — Liquid's primary Workspace navigation surface, listing Pages and Sources as peer entries.
- **Page template** — A reusable starting composition of Blocks, Views, Connections and Automations for one use case; it creates an ordinary Page.
- **PowerBar** — Liquid's command and creation surface for finding Workspace objects, adding Blocks and invoking permitted Workspace or agent actions.
- **Role** — A reusable allow-only composition of permissions assigned to users, groups, agent profiles, sessions or Automations. Anything not allowed is denied by default; explicit deny/blacklist rules are post-MVP.
- **History** — Liquid's attributed and reversible record of Workspace Actions; also called **workspace action history / workspace VCS** when distinguishing it from project-code VCS. Its detailed storage/recovery model remains open in [D-028](docs/decisions/open-decisions.html#d-028) and [D-031](docs/decisions/open-decisions.html#d-031).
- **Liquid replica** — A complete local working copy of an authorized Workspace. Interactive reads and writes never require a remote round trip.
- **Sync Server** — Central service in the replica star topology. It stores and exchanges committed Workspace changes resumably; it does not execute App Blocks or control agents.
- **Headless Liquid replica** — Optional Liquid replica without a user interface. A Workspace must opt in before this replica can sync it; same-host agents then use Liquid's normal permission and action surfaces while user devices are offline.
- **Arrange mode** — The explicit Page state that reveals the responsive grid and resize/reorder handles. Outside Arrange mode, a Page remains a content-first flow.
- **Block SDK** — Versioned contract for reusable built-in and Block Registry Block Types: state/migrations, Views, ports/actions, permissions, lifecycle and responsive behavior. It is for product/community extension, not end-user app creation.
- **Block Registry** — Official distribution catalog for reusable Block Types. Community packages are signed, sandboxed and capability-scoped; the initial model loads no arbitrary third-party native code.
- **App** — Workspace-local interactive functionality with versioned state, declared UI, typed actions and explicit capabilities.
- **App Block** — A Block containing one App. It may expose interactive and code-oriented Views and may be shown through Views on multiple Pages.
- **App SDK** — The user/AI-facing contract for building Workspace-local Apps with minimal code, versioned state, declared UI/actions and explicit capabilities. It is distinct from the Block SDK.
- **App Runtime** — Liquid's local sandbox for App code and declarative UI. Restricted JavaScript/TypeScript is the first target; WASM is a later extension.
- **MCP adapter** — Boundary that maps an external MCP server's capabilities into typed, permissioned Liquid App actions. It may support an App Block or a headless integration.
- **App action** — A typed capability declared by an App, invoked by its UI, a Connection, an Automation or an authorized agent through Liquid's CLI/API.
- **Action** — One recorded Workspace mutation: actor (human/Liquid AI/external agent/Meta-Harness/system), origin (UI/CLI/API/Automation/import/sync), target, operation, before/after-or-patch, permission result, review status, `correlation_id` and revert metadata.
- **Mutation pipeline** — The single permissioned path (validate → permission check → diff → apply → record Action → emit event) every Workspace write takes, including UI, Liquid AI, CLI/API, Automation, App and sync writes. No hidden write bypasses it.
- **Workspace CLI/API** — Liquid's external surface (`liq …` and local API/service) through which agents and integrations discover permitted state and invoke typed reads, mutations and App actions.
- **Liquid AI** — Liquid's built-in Workspace assistant. It uses the same Role, capability, mutation and History boundaries as external agents and reaches CLI agents only through the owning Meta-Harness.
- **External agent** — Any agent or tool, including BYOA and Meta-Harness-orchestrated agents, that uses the Workspace CLI/API. Its effective access is scoped, attributed and reversible.
