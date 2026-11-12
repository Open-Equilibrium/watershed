# Glossary

Canonical terms. Use these exactly; do not introduce synonyms. Tool names are final.

## Platform & strategy

- **Platform** — Watershed as a whole: one AGPL/free-software AI-native work platform for reusable, measurable and reversible agent workflows, composed of three independently usable layers.
- **Platform layer** — One independently usable part of the Watershed platform. There are three: execution, control and workspace/action.
- **Execution layer** — Flow Agent's role: running repeatable, auditable agent workflows.
- **Control layer** — Meta-Harness's role: orchestrating, observing and measuring agents of any harness across agent hosts, leaving their governance to each harness.
- **Workspace/action layer** — Liquid's role: human/agent workspace co-editing with reversible action history.
- **Agent workflow** — A structured unit of AI-agent work that can be run, observed, measured and improved (in Flow Agent, realized as a Flow).
- **Reversible agent action** — An attributed workspace mutation that can be inspected and reverted (see Action, Workspace action history).
- **Wedge** — The first narrow adoption path used to validate and grow the platform. Flow Agent is the developer/open-source execution wedge, Meta-Harness the orchestration wedge, Liquid the long-term workspace/action wedge.
- **AGPL/free-software posture** — The project's commitment to transparent, inspectable, self-hostable software under the repository's `AGPL-3.0-only` license; not a proprietary/open-core monetization stance.
- **Offline after provisioning** — The state in which installed Watershed binaries, approved model/runtime artifacts, required Global Flow configuration, one applicable Runtime binding and local-runtime readiness are already present, so the core local Flow, control and Workspace paths operate with network interfaces disabled. Requested remote-only actions fail visibly when unavailable; sync reports offline/stale state and waits resumably without blocking local work.
- **Topic branch** — Short-lived Git branch for one logical change; can span multiple sessions and is PR'd back to `main`.
- **Git upstream** — Remote-tracking branch a local branch uses by default for pull/push; topic branches must not use `origin/main`.
- **Fast-forward pull** — Fetches the Git upstream and advances the local branch only when it has no commits of its own to merge; diverged branches require a merge or rebase in a terminal.
- **Commit metadata** — Commit subject, body, comment text and trailers; metadata changes must not change file content.
- **Unpublished commit** — Commit not yet pushed or otherwise shared; only these commits may be reworded for metadata corrections.

## Tools

- **Liquid** — Standalone local-first workspace and app-building product built from Pages, Blocks, Views, Sources, Connections and Automations. Every client works through a local replica; optional sync, agent integration and headless execution extend rather than replace that product. CLI binary: `liq`.
- **Flow Agent** — Host-local, CLI-only, Rust-core, script-driven deterministic Flow runtime (not a generic coding agent). M1 is fixture-bounded; productive execution validates Tool calls and requires native self-protection. Guarantees and responsibilities are canonical in [SECURITY.md](SECURITY.md#flow-agent-security-contract). CLI binary: `flow`.
- **Meta-Harness** — Self-contained host-scoped headless control plane over CLI agents on one host (session registry, central config resolution, agent schedules/triggers, artifact indexing, AgentPulse), reachable through local or authenticated remote clients. Runs without Liquid; Liquid is its primary rich UI. CLI binary: `meta`.
- **Execution location** — User-editable label for the host and owning Meta-Harness target used to start or control a session. Technical identifiers remain internal while routing authority is preserved.
- **Runtime binding** — Device-local resolution from a Flow's model/runtime requirements to one concrete provider endpoint and its applicable provider/model identity, optional local artifact/quantization/runtime identity, optional typed provider/audience-bound credential reference and resource policy. Credential material remains in its Flow-owned protected store. The binding is operational host state, not Global Flow configuration or Conversation authority; D-059 owns the exact capability-dependent schema.

## Roles & layers

- **Agentic Engineer** — A technically proficient person who configures Flow Agent Building Blocks and their capability limits, and accepts responsibility for the resulting Flow behavior.
- **Operator** — The person or process that invokes Flow Agent and owns its host identity and runtime data. An operator may be an Agentic Engineer or may only run a predefined Flow.
- **Core** — Shared libraries for Building Blocks, invocation-policy validation and protocol types. Each product owns its runtime and enforcement boundaries.
- **Protocol** — The versioned contract over which the tools communicate (the integration seam).
- **Meta-Agent** — The agent that _operates_ the Meta-Harness; either Liquid-native or BYOA. May propose configuration changes for underlying agents, which take effect only once approved (ADR-0227, planned).
- **BYOA** — "Bring Your Own Agent"; plugging an external agent in as the Meta-Agent.
- **AgentPulse** — Meta-Harness subsystem measuring rework ratio, first-attempt success rate, and cost-per-productive-outcome. Meta-Harness computes/stores the metrics; Liquid only renders them.
- **Adapter** — A Meta-Harness subsystem that translates an external agent's harness (today only pi) into normalized protocol events/commands. Native agent shapes do not leak past the adapter.
- **Agent event** — One event of the in-process session model the embedded Meta-Harness core gives Liquid's agent surface until its service mode (ADR-0195): turns, items, content deltas, requests and usage. Not a Flow Agent runtime event.
- **Partial output** — What a running tool call has written so far, reported as its whole output while it runs; each report replaces the one before it, unlike a content delta, which appends text.

## Flow Agent primitives

- **Building Block** — The flexible, modular and recursive unit of configuration; every Tool, Instruction, Phase and Flow is a building block. Flows can contain flows.
- **Global Flow configuration** — Flow Agent configuration and Building Blocks resolved only from `FLOW_AGENT_HOME` (default `.flow` beneath the user home). Flow Agent does not discover project-local configuration layers; project-specific behavior is expressed by selecting or authoring a Flow. `AGENTS.md` is a separate instruction/context channel without technical configuration authority.
- **Building-block registry** — The resolver for addressable Tools, Instructions, Phases and Flows. In v0 it safely catalogs one-block YAML entries under a configured root, then retains and resolves only the selected Flow's transitive definition closure, once per unique definition.
- **Canonical serialization** — Deterministic UTF-8 JSON of the parsed, semantically validated and registry-resolved selected definition closure; equivalent closures serialize to the same bytes for review, audit and golden tests.
- **Tool** — A Phase-scoped capability with an exact command identity (predefined or own script) and typed accepted parameters. Referencing it only makes it available; the provider decides whether and how often to request it but cannot change that invocation contract. The Engineer trusts its complete implementation and dependency/delegation chain.
- **Policy artifact** — The canonical JSON output produced by `core-policy` for a Tool's validated command and parameter contract. It is not a filesystem, network or process-containment policy. Fixture evaluation proves deterministic invocation decisions, not OS security.
- **Predefined-command registry** — The trusted id-to-executable map used by predefined-command Tools. A script names a `command_id`; Flow Agent resolves it to one executable identity and combines it with the script's literal base `argv` without PATH lookup or shell parsing.
- **Predefined-command Tool** — A Tool that names a fixed command id resolved through the predefined-command registry. Fixtures emulate its result; productive execution sends the verified identity through the Executor boundary.
- **Allowed parameter** — A reviewed parameter spec for a Tool: exact name, typed value shape, required flag and type-specific constraints such as enum values, string pattern/length or integer range. Unknown parameters and values that fail validation are denied before tool launch.
- **Own-script Tool** — A Tool whose reviewed inline `script_body` uses the fixed v0 `posix-sh` contract. Fixtures interpret only their bounded subset; productive execution uses the same Executor boundary as every other Tool.
- **Instruction** — A modular prompt primitive with typed `{{name}}` parameters. A leaf Phase references Instructions and binds their declared values from its complete typed input; an explicitly selected source filename has no authority or naming semantics.
- **Phase** — The recursive workflow unit. A leaf exposes Instructions and Tools to one provider loop and returns a typed result. A composite runs ordered child Phases without its own provider turn and selects one child's result. Either may use a bounded declarative loop.
- **Transition** — An ordered inline forward-sibling rule owned by a Flow or composite Phase. It selects a later sibling only when an exact typed equality predicate matches the successful source result; otherwise execution falls through.
- **Flow** — A fully configured deterministic orchestration of ordered Phases and optional subflows. Whole successful results pass forward by default; a Flow is itself a building block.
- **Subflow** — A flow used inside another flow.
- **Flow execution plan** — The finite typed, deterministic and side-effect-free description of resolved Flow invocations, lifecycle boundaries, context inputs and Tool intents consumed by runtime apply.
- **Execution request** — One versioned, bounded request for exactly one validated Tool invocation: executable, arguments, working directory, environment, fixed runtime limits and identity-bound descriptors of protected Flow objects, bound by a canonical digest.
- **Executor** — A Flow Agent execution component that validates one Execution request, waits for Start, establishes self-protection, supervises one Tool invocation and returns a bounded result with enforcement receipt or typed error. Observed cleanup is not a universal hostile-descendant guarantee.
- **Default Executor** — The official `flow-executor` companion, using Linux Bubblewrap/seccomp or macOS Seatbelt to establish native self-protection before Tool launch. Exact release targets and verification status are in [PLATFORMS.md](PLATFORMS.md).
- **Custom Executor** — An administrator-selected implementation of the same Executor protocol on a supported Flow Agent host. Flow Agent documents the protocol and reports integration failures, but does not certify or guarantee a third party's compatibility, security or operation.
- **Meta-Harness agent executor** — Meta-Harness's host-local component for launching and supervising whole CLI agent processes. It is not a Flow Agent Executor and never selects, configures or manages one.
- **Fixture executor** — The M1 in-process executor enabled only by the canonical fixture profile; it returns deterministic model/Tool doubles and is not a provider, subprocess runner or POSIX shell.
- **Compatibility probe** — A non-authoritative Executor handshake and self-test that can detect known protocol, platform and setup mismatches. Passing it is useful diagnostics, never a compatibility or security guarantee.
- **Enforcement receipt** — The canonical bounded Executor evidence persisted with a terminal Tool attempt. It identifies the applied backend/platform, active self-protection and exact canonical request-policy digest; structural validation does not certify a Custom Executor's truthfulness.
- **Policy emulation** — M1's deterministic in-process evaluation of modeled policy decisions. It proves runtime correctness contracts, not OS isolation.
- **Sandbox** — An execution environment whose selected restrictions are enforced by an operating-system, container or VM mechanism. The term alone promises neither whole-host containment nor a particular set of restrictions, so always qualify it: Flow's native self-protection through its Executor ([SECURITY.md](SECURITY.md#flow-agent-security-contract)); an external sandbox around Flow Agent; a harness's own sandbox, which Meta-Harness leaves unchanged (ADR-0252); the macOS App Sandbox, compared for Flow but not selected (ADR-0172); or Liquid's App Runtime.
- **Sandbox backend** — The operating-system, container or virtual-machine mechanism enforcing selected restrictions. Flow's native backends use Linux Bubblewrap/seccomp or macOS Seatbelt for self-protection; a backend does not understand Building Blocks by itself.
- **Container** — A Sandbox form that isolates processes while sharing the host kernel. The Default Executor uses none, a Custom Executor may, and it is not automatically a sufficient security boundary.
- **Virtual machine (VM)** — An isolated guest system with its own kernel; higher isolation usually costs more startup time and operational complexity.
- **OS isolation** — Restrictions enforced by an operating-system mechanism rather than prompt instructions or application intent. Flow Agent promises only its selected self-protection boundary, not general filesystem, process or network containment.

- **Self-protection** — The mandatory native restriction against direct modification of this installation's protected Flow-owned objects by Tools and newly launched children. Independent unconfined services are outside this boundary; see [SECURITY.md](SECURITY.md#flow-agent-security-contract).

## Flow Agent runtime surfaces

- **Access-control list (ACL)** — Operating-system permission entries that grant or deny named users or groups access beyond ordinary file-mode permissions. Installation prerequisites are in [PLATFORMS.md](PLATFORMS.md#installation-prerequisites).

- **flow-value-v0** — The closed, explicitly tagged and provider-neutral M1.1 value contract shared by root input, Phase input/results, Instruction parameters, Tool parameters and Tool outputs. It permits no implicit coercion or floating-point values; large or binary values use immutable session-object references.
- **flow-run-input-v0** — The versioned run-input document containing one complete `flow-value-v0` value for the selected root Flow; its canonical contract is in `PROTOCOL.md`.
- **flow-tool-result-v0** — The fixed Tool-result envelope for status, stdout and stderr flow values, with exit code only when observed; its canonical contract is in `PROTOCOL.md`.
- **flow-m11-performance-evidence-v0** — Release-mode Ubuntu JSONL schema for the finite M1.1 observational workload set; it records exact inputs, raw samples and unadjusted timing and RSS aggregates without performance thresholds.
- **flow-m12-executor-startup-v0** — Release-mode native-host JSONL schema for the fixed M1.2 one-shot Executor startup observation; its canonical workload and evidence policy live in [`M1_2_STARTUP_EVIDENCE.md`](flow-agent/benchmarks/M1_2_STARTUP_EVIDENCE.md), without a timing threshold.
- **Conversation** — The complete user-facing work history: one append-only tree of navigable entries and all Runs that belong to it. Continuing from an earlier entry creates a branch without deleting descendants.
- **Portable continuation** — A verified, continuation-eligible terminal Conversation checkpoint continued on another device or Runtime binding as a new child Run. It rebinds authority and capabilities, never replays prior effects, and is distinct from exact interrupted-Run recovery.
- **Conversation entry** — One `flow-conversation-entry-v1` history node that links to its parent and to a committed point in one owned linear Run, and binds the terminal recovery snapshot used for compact continuation.
- **Run** — One complete execution of a Flow inside a Conversation, from Flow start through its Phases to completion or failure. Each continuation or branch starts a new Run. Its technical Run ID is stored as lowercase path-safe `run_session_id`; events use it as `session_id`, and exact replay, Tail or interrupted-Run recovery address it together with its owning `conversation_id`.
- **Transcript** — The messages and runtime events reachable on a selected conversation branch; the complete tree retains every branch.
- **Durable session history** — The append-only conversation tree plus every owned linear run and referenced immutable source artifact. Provider-context optimization and branch navigation never delete it or roll back external effects.
- **Run Log** — The canonical versioned append-only `run-log.jsonl` for one Run. Its first record preserves the definition identity and hashes required for Resume; later records durably track provider and Tool dispatch attempts, and per-Tool Run Logs project only Tool records rather than using separate files.
- **Recovery snapshot** — One bounded, versioned `recovery.jsonl` prefix for a productive Run. Its synchronized records preserve enough deterministic orchestration state and durable-result references to resume the exact Run without repeating completed provider or Tool calls; its terminal record also preserves compact cumulative state for a child Run.
- **Uncertain attempt** — A durable provider or Tool intent without a committed terminal result. Flow Agent never redispatches it automatically; `PROTOCOL.md` owns reconciliation behavior.
- **Provider reply invalid** — A provider reply that ends but breaks the Responses format. It closes the attempt as `provider_reply_invalid`, is never retried, and differs from both `provider_error` and an uncertain attempt; its contract is in `PROTOCOL.md`.
- **Tool attempt stage** — The `preflight` or `started` stage recorded with a cancelled Tool attempt, stating whether `tool.started` was committed; its contract is in `PROTOCOL.md`.
- **Process cleanup unknown** — The recorded mark that a Tool's processes may outlive its cleanup after the Executor's answer. The attempt keeps that answer's status, output and exit code, and the Run stops with the same reason; its contract is in `PROTOCOL.md`.
- **Run bundle** — One Run's event/context/recovery segments, Run Log including its definition metadata, and immutable objects.
- **Conversation bundle** — One conversation history plus all owned Run bundles.
- **Host-local session ownership leases** — Exclusive operating-system file leases in the private user-global Workspace store. `PROTOCOL.md` defines their ownership and acquisition contract. Process exit releases leases; none is durable or transferable across hosts.
- **Stash operation token** — The unique suffix in the message of a stash Liquid saves, such as `Liquid: changes left on main · 7f3a9c0123456789`, which identifies that save independently of the stash list's order.
- **Session branch** — One path through a pi session's tree of entries, which "Branch from here" and a revert start; not a Git branch, which keeps the name "branch" in Liquid.
- **Session marker** — The persistent `session.lock` observability leaf inside a run directory. It is not ownership authority and cannot grant or revoke the lease.
- **Reservation orphan** — An empty run artifact retained and inventoried when rollback cannot prove identity-bound deletion. Its cleanup failure remains visible, and compare-then-unlink never removes it.
- **Event segment** — One append-only canonical JSONL file within a run bundle; rotation bounds individual I/O without starting a new run or resetting sequence/budgets.
- **Resolved flow state** — The current Flow invocation and Phase execution stack, active Instructions and available Tools, typed input/result and runtime state exposed by defined interfaces.
- **Provider context** — The deterministic, bounded projection compiled from resolved flow state and narrowly selected durable history for one model turn; not the full transcript.
- **Context profile** — A versioned deterministic contract for provider-context ordering, budgeting, tokenization/estimation, projections, hashing and cache boundaries; M1 exposes only `flow-context-v0`.
- **Context manifest** — The reproducible per-provider-turn record of the context profile, budget inputs, ordered included/projected sources, omissions, cache boundaries, token estimate and final context hash.
- **Runtime event** — One normalized event Flow Agent emits over its public contract (see `PROTOCOL.md`); the same events feed JSONL mode, future RPC mode and the session store.
- **JSONL event stream** — Flow Agent's headless mode that streams newline-delimited JSON runtime events to stdout for automation/CI/consumers.
- **RPC mode** — Flow Agent's designed-for bidirectional stdin/stdout control mode. [PROTOCOL.md](PROTOCOL.md) defines its initial command/request shape; runtime events remain the public event contract.
- **Session store** — Flow Agent's conversation trees and owned run bundles in a canonical-path-keyed private user-global Workspace store. Runtime state only; **not** a project VCS/history engine.
- **Live-event notification** — A bounded, best-effort wake-up reporting a session's earliest pending and highest committed sequences; it carries no event payload, and receivers replay events after their cursor from the authoritative session log.
- **Persistence-before-notification** — The local guarantee that one serial session writer successfully appends a canonical event to the authoritative log before updating its high-watermark and attempting a live-event notification; physical `fsync` follows separate bounded durability checkpoints.
- **Flow registry** — The name/id index used by `flow run <name>` and by `flow chat`, which reads one nonblank stdin Flow reference with an optional leading `/`, runs it, then exits.
- **Fixture workspace** — A checked-in test workspace whose explicit canonical fixture profile selects the fixture executor and deterministic stub model.
- **Flow definition ID** — The registry/building-block id of a Flow definition; carried in event payloads as `flow_definition_id`.
- **Runtime flow invocation ID** — The `flow_id` assigned to one root-flow or subflow invocation in a session; distinct from the Flow definition ID and linked to a parent by `parent_flow_id`.
- **Stub model** — A deterministic model double used by tests so golden event streams are byte-stable in CI.
- **Golden flow** — A checked-in flow fixture with a deterministic expected event stream used for capture-and-diff validation.
- **Golden event stream** — A checked-in JSONL event stream with fixed fixture IDs, timestamps, sequence values and canonical event JSONL bytes per `PROTOCOL.md`.
- **Environment allowlist** — A tool-scoped policy-artifact field; its M1 availability and restrictions are defined in [`SECURITY.md`](SECURITY.md).
- **Smoke-flow** — The smallest golden flow: one phase, one tool and one instruction, used as the first localizable gate.
- **Hello-flow** — The showcase golden flow that exercises multiple Phases, scoped Instructions/Tools and subflow reuse.
- **Sandbox-negative fixture** — A tiny flow that intentionally attempts a forbidden operation and must be rejected.
- **HTML render gate** — The CI validation that browser-renders self-contained HTML docs; `pnpm run docs:render-check` invokes `scripts/check-html-render.mjs` at `1440x900` and `390x844`.

## Liquid primitives

- **Workspace** — Top-level owner of Pages, Blocks, Sources, Connections, Automations, Roles, History and settings.
- **Page** — Liquid's authored surface: an ordered composition of Block Views with responsive layout metadata. A Page starts as an empty flow and may later be arranged on a grid.
- **Block** — A canonical, typed and addressable unit of content or functionality, such as text, database, code, formula, media, whiteboard, App or Chat. A text toggle is formatting inside a Text Block, not a Block Type.
- **Block Type** — The versioned behavior, state contract, supported Views, ports/actions, permissions and responsive capabilities shared by Blocks of one kind.
- **View** — One placement or representation of a Block. Several Views may expose the same Block on different Pages and stay synchronized because they share canonical Block state. Duplicating creates a new Block; adding a View does not.
- **Connection** — An explicit typed and permissioned data/action relation among Blocks and Sources. It addresses stable objects and ports independently of active View or layout; visual proximity never grants access.
- **Automation** — A Workspace-owned event/condition/action rule over Liquid objects and permitted external actions. It is distinct from Meta-Harness agent scheduling and orchestration.
- **Source** — An external or shared resource addressable from a Workspace. Sources appear beside Pages in the Explorer and use the same open/navigation flow, but are not authored Pages internally.
- **Explorer** — Liquid's primary Workspace navigation surface, listing Pages and Sources as peer entries.
- **Page template** — A reusable starting composition of Blocks, Views, Connections and Automations for one use case, saved from Arrange mode; it creates an ordinary Page, asking by default for the Chats it shows (ADR-0241).
- **PowerBar** — Liquid's command and creation surface for finding Workspace objects, adding Blocks and invoking permitted Workspace or agent actions. Without a prefix it searches and jumps; `/` adds a Block and `>` lists actions, as the approved [Pages mockups](docs/design/mockups/liquid-workspace-pages.html) show; `@` is kept for people (ADR-0236).
- **Slash commands** — The commands `/` lists in a menu at the caret in an empty Text Block, as in Notion and T3 Code: the Blocks to add, filtered by what follows `/`. The PowerBar's `/` lists the same ([Pages mockups](docs/design/mockups/liquid-workspace-pages.html)).
- **Liquid key** — Caps Lock when the user opts in: a tap opens Liquid from any app, holding it in Liquid reveals shortcuts and ⇪ plus a key runs a Liquid command.
- **Shortcut profile** — Liquid's set of shortcuts: Liquid key, Standard (⌘ only) or Custom.
- **Reveal key** — The key whose hold shows contextual shortcut hints: the Liquid key while it is on, ⌘ otherwise.
- **Role** — A reusable allow-only composition of permissions assigned to users, groups, agent profiles, sessions or Automations. Anything not allowed is denied by default; explicit deny/blacklist rules are post-MVP. Editable templates: Can view, Can edit and Can manage.
- **Protected** — A Page, Block or Source that stops inheriting rights from its area or Page, so only Roles allowed on it may change it; the allow-only way to restrict one part of an editable Page (ADR-0235).
- **History** — Liquid's attributed and reversible record of Workspace Actions; also called **workspace action history / workspace VCS** when distinguishing it from project-code VCS. A revert records a new reversing Action; Restore version returns a Page to an earlier save point (ADR-0235).
- **Agent surface** — Liquid's view of one agent session: the sidebar, the Chat, Trajectory and Diff tabs, the widget strip, the input area and the status bar; ADR-0239 plans to fold it into Chats.
- **Start page** — What the agent surface shows before a session's first message: the project's recent sessions to continue and chips to set the new one up.
- **Input area** — The fixed-height area at the bottom of the agent surface: the composer, or in its place an agent question.
- **Trajectory** — Liquid's step-by-step view of an agent session: model responses, tool calls, tokens, timing and the context the model saw. A session can branch from any step, and two branches can be compared side by side.
- **Git integration** — Git operations on the projects agents work in — diffs, branches, commits, push, pull requests and worktrees — run by Meta-Harness on its host and presented in Liquid (ADR-0190). It operates the project's own Git repository and is not a Watershed VCS; Liquid's own VCS is History.
- **CRDT** — Conflict-free replicated data type: a data structure whose independently edited copies merge deterministically to the same state. Liquid sync uses an established CRDT library (ADR-0182): Automerge 3 (ADR-0238).
- **Rust–Flutter bridge** — The asynchronous call and stream boundary between Liquid's Flutter UI and its Rust core. It carries bounded deltas, never whole Pages or Databases (ADR-0182); it uses flutter_rust_bridge v2 (ADR-0186).
- **Liquid replica** — A complete local working copy of an authorized Workspace. Interactive reads and writes never require a remote round trip.
- **Replica key name** — The random name, kept in a small file in the replica's folder, under which the operating system's keychain files the replica's database key, so the key follows a moved folder (ADR-0275).
- **Sync Server** — Central service in the replica star topology. It stores and exchanges committed Workspace changes resumably; it does not execute App Blocks or control agents.
- **Harness handoff** — Continuing a conversation in another harness as a new session that receives the earlier conversation as bounded text, without native tool or reasoning state (ADR-0231).
- **Area** — A part of a Workspace that Roles, replication and encryption treat as one unit. Its area key is wrapped for every device and headless replica allowed in it (ADR-0224, ADR-0228).
- **Agent host** — Where Meta-Harness runs agents: by default the operating-system account it is installed on (ADR-0254), or a separate machine, VM, container or account the person sets up and links. A default agent host has no replica of its own: its agents use the person's Liquid through the Workspace CLI/API (ADR-0255).
- **Subagent** — A thread an agent starts through Meta-Harness's orchestration tools, in any harness, with any model and on any agent host, which reports back to it as messages; its authority never exceeds its parent's grant (ADR-0253).
- **Grant** — What a person signs to let an agent act for them in a Workspace: an area, actions and an expiry, never more than that person may do; revoking it makes the Sync Server refuse the agent's changes (ADR-0223).
- **Agent-sessions area** — The area where Meta-Harness mirrors agent session records at turn boundaries, so another of the person's agent hosts can continue a session (ADR-0229).
- **Replay-safe** — A tool that may run again after a crash without a second effect, such as pi's own reading tools or a Flow Agent Tool declared so; other tools report "interrupted" instead (ADR-0229).
- **Shown session** — The stored session a Liquid window displays. It holds a shared lock, so deleting that session is refused while any window of the project shows it (ADR-0271). Unlike a running agent, it does not block reverts or commits.
- **Session record** — The folder “Export record” writes into Downloads for one stored session: pi's session file unchanged, `record.json` with its turns, and a `README.txt` of what it leaves out (ADR-0257).
- **Checkpoint ref** — A Git ref under Meta-Harness's own namespace, `refs/watershed/checkpoints/<session>/<turn>/<start|end>`, that points at a commit of the agent project's files at a turn's start or end, to show the turn's changes and to revert the files to it (ADR-0220, ADR-0234).
- **Context track** — The Trajectory's column that draws each step's context against the model's window: exact at each request's prompt tokens, estimated between them, with pi's compaction point.
- **Compliance replica** — A replica a company runs with its own key for search, legal hold and data-loss checks (ADR-0228).
- **Recovery key** — The second way to regain a Workspace's keys when no signed-in device can approve a new one. Area keys are also wrapped for it, and the person or their company holds it (ADR-0228).
- **Headless replica** — A Liquid replica without UI, on a server for Automations and schedules or on a separate agent host. It holds only the Workspaces enabled for it and, once selective replication follows the complete M3 replicas, only the areas its Roles allow (ADR-0255).
- **Text Block** — One paragraph, heading, list item, to-do or quote; ⏎ ends it and starts the next.
- **Table** — The Block of typed fields and rows, kept in its home Page's document and shown through Views such as Table, Board and Calendar. The core calls it the Database Block (ADR-0244).
- **Calculation Block** — A Block of lines that each pair a label with a value or formula, laid out as labels left and values right, for reports in Excel's layout (ADR-0244).
- **Section** — Consecutive Text Blocks up to `---`, a Block of another type or the Page's edge; Arrange mode places it as one tile, inside which its Text Blocks can be arranged in columns. It is layout only and owns no content, rights or Views (ADR-0235).
- **Chat** — Liquid's one kind of conversation, as in WhatsApp: one-to-one or a group, whose participants are people or agents alike; a Block that Pages can show. A planned replacement for separate agent sessions and projects (ADR-0239, ADR-0241).
- **Inbox** — Liquid's one list of what needs the person, from every Chat and Table: agents' approvals and questions, mentions, direct messages and tasks due soon, ordered by when they are due (ADR-0241).
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
- **Liquid AI** — Liquid's built-in Workspace assistant. It runs as a built-in agent profile on an agent host, limited to Liquid actions, and uses the same Role, capability, mutation and History boundaries as external agents (ADR-0229).
- **External agent** — Any agent or tool, including BYOA and Meta-Harness-orchestrated agents, that uses the Workspace CLI/API. Its effective access is scoped, attributed and reversible.
