# Glossary

Canonical terms. Use these exactly; do not introduce synonyms. Tool names are final.

## Platform & strategy

- **Platform** — Watershed as a whole: one AGPL/free-software AI-native work platform for reusable, measurable and reversible agent workflows, composed of three independently usable layers.
- **Platform layer** — One independently usable part of the Watershed platform. There are three: execution, control and workspace/action.
- **Execution layer** — Loop Agent's role: running repeatable, auditable agent workflows.
- **Control layer** — Meta-Harness's role: controlling, observing, measuring and governing many agents.
- **Workspace/action layer** — Liquid's role: human/agent workspace co-editing with reversible action history.
- **Agent workflow** — A structured unit of AI-agent work that can be run, observed, measured and improved (in Loop Agent, realized as a Loop).
- **Reversible agent action** — An attributed workspace mutation that can be inspected and reverted (see Action, Workspace action history).
- **Wedge** — The first narrow adoption path used to validate and grow the platform. Loop Agent is the developer/open-source execution wedge, Meta-Harness the team/control/governance wedge, Liquid the long-term workspace/action wedge.
- **AGPL/free-software posture** — The project's commitment to transparent, inspectable, self-hostable software under the repository's `AGPL-3.0-only` license; not a proprietary/open-core monetization stance.
- **Topic branch** — Short-lived Git branch for one logical change; can span multiple sessions and is PR'd back to `main` (ADR-0046).
- **Git upstream** — Remote-tracking branch a local branch uses by default for pull/push; topic branches must not use `origin/main` (ADR-0048).
- **Commit metadata** — Commit subject, body, comment text and trailers; metadata changes must not change file content (ADR-0048).
- **Unpublished commit** — Commit not yet pushed or otherwise shared; only these commits may be reworded for metadata corrections (ADR-0048).

## Tools

- **Liquid** — Standalone local-first workspace and app-building product (Pages, Blocks, Views, Sources and automations) with an internal workspace action history/VCS and a workspace CLI/API. Useful without Loop Agent or Meta-Harness; optionally syncs workspace data and presents connected Meta-Harness instances in one UI. CLI binary: `liq` (ADR-0013).
- **Loop Agent** — Host-local, CLI-only, Rust-core, script-driven, event-based deterministic agent-loop harness (not a generic coding agent). CLI binary: `loop` (ADR-0013).
- **Meta-Harness** — Self-contained host-scoped headless control plane over CLI agents on one host (session registry, central config resolution, scheduling/automations, artifact indexing, AgentPulse), reachable through local or authenticated remote clients. Runs without Liquid; Liquid is its primary rich UI. CLI binary: `meta` (ADR-0013).
- **Pi Agent** — The Pi CLI agent integration target. Use the full term "Pi Agent" in docs; avoid bare "Pi" except when quoting an external CLI/product name.

## Roles & layers

- **Core** — Shared libraries used by all tools: building-block/script format, identity/permissions, policy→sandbox compiler and config/protocol helpers.
- **Protocol** — The versioned contract over which the tools communicate (the integration seam).
- **Meta-Agent** — The agent that _operates_ the Meta-Harness; either Liquid-native or BYOA. May reconfigure underlying agents under policy + audit control.
- **BYOA** — "Bring Your Own Agent"; plugging an external agent in as the Meta-Agent.
- **AgentPulse** — Meta-Harness subsystem measuring rework ratio, first-attempt success rate, and cost-per-productive-outcome. Meta-Harness computes/stores the metrics; Liquid only renders them.
- **Adapter** — A Meta-Harness subsystem that translates an external agent (Codex CLI, Claude Code, Pi Agent, etc.) into normalized protocol events/commands. Native agent shapes do not leak past the adapter.

## Loop Agent primitives

- **Building Block** — The flexible, modular and recursive unit of configuration; every Tool, Instruction, Phase and Loop is a building block. Loops can contain loops.
- **Building-block registry** — The resolver for addressable Tools, Instructions, Phases, Loops and Connections. In v0 it safely catalogs one-block YAML entries under a configured root, then retains and resolves only the selected Loop's transitive definition closure, once per unique definition.
- **Canonical serialization** — Deterministic UTF-8 JSON of the parsed, semantically validated and registry-resolved selected definition closure; equivalent closures serialize to the same bytes for review, audit and golden tests.
- **Tool** — A capability with an exact command identity (predefined or own script) and declared parameter, path and network boundaries. Nothing outside the declared command is permitted.
- **Policy artifact** — The canonical JSON output produced by `core-policy` to show tool-scoped capabilities for a target sandbox backend, including deterministic object-key and array ordering. M1 enforces compatible policy deterministically in process; OS sandbox backends are post-M1.
- **Predefined-command registry** — The trusted id-to-executable map used by predefined-command Tools. A script names a `command_id`; Loop Agent resolves it to one executable identity and combines it with the script's literal base `argv` without PATH lookup or shell parsing.
- **Predefined-command Tool** — A Tool that calls a fixed command id declared by the script, resolved through the predefined-command registry and constrained by policy.
- **Allowed parameter** — A reviewed parameter spec for a Tool: exact name, typed value shape, required flag and type-specific constraints such as enum values, string pattern/length or integer range. Unknown parameters and values that fail validation are denied before tool launch.
- **Own-script Tool** — A Tool whose reviewed inline `script_body` uses the fixed v0 `posix-sh` contract. M1 execution and later sandboxing are defined in the [Loop Agent V-Spec](docs/concept/V-Spec_LoopAgent.html) and [`SECURITY.md`](SECURITY.md).
- **Instruction** — A modular prompt primitive (`id`, `name`, `prompt`). Carries no phase binding or tool ownership; phases reference instructions and tools.
- **Connection** — A declared relation between building blocks, data sources, events or outputs. Connections make data/control flow explicit instead of hiding it in agent-specific terminology.
- **Phase** — A workflow stage; declares the tools and instructions available within it and contains ordered steps. Authored as a script; a visual graph is a view over that script.
- **Loop** — A fully configured, deterministic AI-native process: a state machine composed of phases and building blocks (1…n agents). A loop is itself a building block.
- **Subloop** — A loop used inside another loop.

## Loop Agent runtime surfaces

- **Session** — One Loop Agent run, identified by a lowercase path-safe `session_id` token per `PROTOCOL.md`; the unit that is started, resumed, tailed and persisted.
- **Transcript** — The ordered record of a session's messages and runtime events; part of durable session history and reconstructable by replay.
- **Durable session history** — The complete append-only session event history plus referenced source artifacts; its events are authoritative for replay, while the full history is authoritative for resume, audit, debugging and future retrieval. Provider-context optimization never deletes either.
- **Session bundle** — All session-owned event and context-manifest segments, immutable hash-addressed objects and definition metadata; export and deletion treat them as one unit.
- **Event segment** — One append-only canonical JSONL file within a session bundle; segment rotation bounds individual I/O without starting a new session or resetting sequence/budgets.
- **Resolved loop state** — The current loop invocation, phase, step, active instructions/tools, connections and runtime state, plus values exposed by defined runtime interfaces.
- **Provider context** — The deterministic, bounded projection compiled from resolved loop state and narrowly selected durable history for one model turn; not the full transcript.
- **Context profile** — A versioned deterministic contract for provider-context ordering, budgeting, tokenization/estimation, projections, hashing and cache boundaries; M1 exposes only `loop-context-v0`.
- **Context manifest** — The reproducible per-provider-turn record of the context profile, budget inputs, ordered included/projected sources, omissions, cache boundaries, token estimate and final context hash.
- **Runtime event** — One normalized event Loop Agent emits over its public contract (see `PROTOCOL.md`); the same events feed JSONL mode, future RPC mode and the session store.
- **JSONL event stream** — Loop Agent's headless mode that streams newline-delimited JSON runtime events to stdout for automation/CI/consumers.
- **RPC mode** — Loop Agent's designed-for bidirectional stdin/stdout control mode. ADR-0055 selects the initial command/request shape; runtime events remain the public event contract.
- **Session store** — Loop Agent's local append-only transcript persistence (e.g. `.loop/sessions/<session_id>.jsonl`). Runtime state only; **not** a project VCS/history engine.
- **Live-event notification** — A bounded, best-effort wake-up reporting a session's earliest pending and highest committed sequences; it carries no event payload, and receivers replay events after their cursor from the authoritative session log.
- **Persistence-before-notification** — The local guarantee that one serial session writer successfully appends a canonical event to the authoritative log before updating its high-watermark and attempting a live-event notification; physical `fsync` follows separate bounded durability checkpoints.
- **Loop registry** — The name/id index used by `loop run <name>` and interactive slash commands such as `/hello-loop` inside `loop chat` to resolve a loop definition without requiring a path.
- **Fixture workspace** — A checked-in test workspace for a golden loop; ADR-0041 defines how it points Loop Agent at the fixture registry and deterministic stub-model profile.
- **Loop definition ID** — The registry/building-block id of a Loop definition; carried in event payloads as `loop_definition_id`.
- **Runtime loop invocation ID** — The `loop_id` assigned to one root-loop or subloop invocation in a session; distinct from the Loop definition ID and linked to a parent by `parent_loop_id`.
- **Stub model** — A deterministic model double used by tests so golden event streams are byte-stable in CI.
- **Golden loop** — A checked-in loop fixture with a deterministic expected event stream used for capture-and-diff validation.
- **Golden event stream** — A checked-in JSONL event stream with fixed fixture IDs, timestamps, sequence values and canonical event JSONL bytes per `PROTOCOL.md`.
- **Environment allowlist** — A tool-scoped policy-artifact field; its M1 availability and restrictions are defined in [`SECURITY.md`](SECURITY.md).
- **Network allow entry** — A typed CIDR/IP egress rule with transport and port; the only v0 way to declare network access. M1 Linux-target deterministic policy rejects non-empty allowlists until D-046 selects a positive egress backend.
- **Protected path** — A path pattern denied by default even inside a declared root unless a loop explicitly grants it.
- **Protected-path grant** — A tool-scoped exception that removes the protected-path deny only when the path is still inside that tool's declared read/write scope.
- **Smoke-loop** — The smallest golden loop: one phase, one tool and one instruction, used as the first localizable gate.
- **Hello-loop** — The showcase golden loop that exercises phases, scoped instructions/tools, connections and subloop reuse.
- **Sandbox-negative fixture** — A tiny loop that intentionally attempts a forbidden operation and must be rejected.
- **HTML render gate** — The CI validation that browser-renders self-contained HTML docs; `pnpm run docs:render-check` invokes `scripts/check-html-render.mjs` at `1440x900` and `390x844` (ADR-0043, ADR-0045).

## Liquid primitives

- **Workspace** — Top-level container for Pages, Sources, settings and action history.
- **Page** — Liquid's top-level authored surface: an ordered composition of Blocks with responsive layout metadata. A Page starts as an empty flow and may be arranged on a grid; there is no second authored-surface object or mode.
- **Block** — A typed, addressable unit of content or functionality placed on a Page, such as text, database, code, formula, script, media, whiteboard or agent status. A text toggle is formatting inside a Text Block, not a Block type. Do not call Liquid Blocks "tools."
- **Block Type** — The behavior, state contract, supported Views, connection ports, permissions and responsive capabilities shared by Blocks of one kind.
- **View** — One representation or interaction surface over a Block's state and capabilities. A Block owns its Views; changing View does not create or replace the Block.
- **Connection** — An explicit typed, permissioned data/control relation between Blocks. It addresses Blocks and their ports independently of the active View; visual proximity never grants data access.
- **Source** — An external or shared data input addressable from a workspace. A Source is not a Page or a generic container for Liquid content.
- **Sync host** — Optional service that exchanges committed Liquid workspace changes with local replicas resumably. It is independent of Meta-Harness and is not Liquid's interactive working store.
- **Arrange mode** — The explicit Page layout state that reveals a responsive grid and resize/reorder handles. Outside Arrange mode, a Page remains a content-first flow.
- **Block SDK** — The contract for built-in and third-party Block Types, including state serialization/migrations, Views, connections/ports/actions, permissions and responsive behavior. ADR-0040 still limits the MVP to a fixed trusted Block palette plus script-as-compute.
- **Workspace action history / workspace VCS** — Liquid's internal record of workspace mutations, enabling attribution, diff and revert. A VCS over Liquid's own workspace data — **not** a project-code VCS and not Loop Agent's session store. Its history and recovery model remain open in [D-028](docs/decisions/open-decisions.html#d-028) and [D-031](docs/decisions/open-decisions.html#d-031).
- **Action** — One recorded workspace mutation: actor (human/liquid-ai/external-agent/meta-harness/system), origin (ui/cli/api/automation/import/sync), target, operation, before/after-or-patch, permission result, review status, correlation_id and revert metadata.
- **Mutation pipeline** — The single permissioned path (validate → permission check → diff → apply → record action → emit event) every workspace write takes, regardless of whether it originates from the UI, Liquid AI, the CLI/API, sync or an external agent. No hidden writes bypass it.
- **Workspace CLI/API** — Liquid's external surface (`liq …` CLI and local API/service) through which external agents and tools read and edit workspace data via the mutation pipeline.
- **Liquid AI** — Liquid's built-in workspace assistant; uses the same mutation/action-history pipeline as external agents and reaches CLI agents only through an owning Meta-Harness public surface.
- **External agent** — Any agent or tool (incl. BYOA, Meta-Harness-orchestrated) that reads/edits a Liquid workspace through the workspace CLI/API; writes are scoped, attributed and revertible.
