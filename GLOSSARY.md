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
- **Offline after provisioning** — The state in which installed Watershed binaries, approved model/runtime artifacts, required Global Flow configuration, one applicable Runtime binding and local-runtime readiness are already present, so the core local Flow, control and Workspace paths operate with network interfaces disabled. Requested remote-only actions fail visibly when unavailable; sync reports offline/stale state and waits resumably without blocking local work.
- **Topic branch** — Short-lived Git branch for one logical change; can span multiple sessions and is PR'd back to `main` (ADR-0046).
- **Git upstream** — Remote-tracking branch a local branch uses by default for pull/push; topic branches must not use `origin/main` (ADR-0048).
- **Commit metadata** — Commit subject, body, comment text and trailers; metadata changes must not change file content (ADR-0048).
- **Unpublished commit** — Commit not yet pushed or otherwise shared; only these commits may be reworded for metadata corrections (ADR-0048).

## Tools

- **Liquid** — Standalone local-first workspace and app-building product built from Pages, Blocks, Views, Sources, Connections and Automations. Every client works through a local replica; optional sync, agent integration and headless execution extend rather than replace that product. CLI binary: `liq` (ADR-0013).
- **Flow Agent** — Host-local, CLI-only, Rust-core, script-driven deterministic Flow runtime (not a generic coding agent). M1 is fixture-bounded; M1.1 adds practical provider/process execution and M1.2 adds OS isolation. CLI binary: `flow` (ADR-0013).
- **Meta-Harness** — Self-contained host-scoped headless control plane over CLI agents on one host (session registry, central config resolution, agent schedules/triggers, artifact indexing, AgentPulse), reachable through local or authenticated remote clients. Runs without Liquid; Liquid is its primary rich UI. CLI binary: `meta` (ADR-0013).
- **Execution location** — User-editable label for the host and owning Meta-Harness target used to start or control a session. Technical identifiers remain internal while routing authority is preserved.
- **Runtime binding** — Device-local resolution from a Flow's model/runtime requirements to one concrete provider endpoint and its applicable provider/model identity, optional local artifact/quantization/runtime identity, optional typed provider/audience-bound credential reference and resource policy. Credential material remains in its Flow-owned protected store. The binding is operational host state, not Global Flow configuration or Conversation authority; D-059 owns the exact capability-dependent schema.

## Roles & layers

- **Agentic Engineer** — A technically proficient person who configures Flow Agent Building Blocks and their capability limits, and accepts responsibility for the resulting Flow behavior.
- **Operator** — The person or process that invokes Flow Agent and owns its host identity and runtime data. An operator may be an Agentic Engineer or may only run a predefined Flow.
- **Core** — Shared libraries used by all tools: building-block/script format, identity/permissions, policy→sandbox compiler and config/protocol helpers.
- **Protocol** — The versioned contract over which the tools communicate (the integration seam).
- **Meta-Agent** — The agent that _operates_ the Meta-Harness; either Liquid-native or BYOA. May reconfigure underlying agents under policy + audit control.
- **BYOA** — "Bring Your Own Agent"; plugging an external agent in as the Meta-Agent.
- **AgentPulse** — Meta-Harness subsystem measuring rework ratio, first-attempt success rate, and cost-per-productive-outcome. Meta-Harness computes/stores the metrics; Liquid only renders them.
- **Adapter** — A Meta-Harness subsystem that translates an external agent (Codex CLI, Claude Code or a future agent) into normalized protocol events/commands. Native agent shapes do not leak past the adapter.

## Flow Agent primitives

- **Building Block** — The flexible, modular and recursive unit of configuration; every Tool, Instruction, Phase and Flow is a building block. Flows can contain flows.
- **Global Flow configuration** — Flow Agent configuration and Building Blocks resolved from one non-Workspace global authority. Flow Agent does not discover project-local configuration layers; project-specific behavior is expressed by selecting or authoring a Flow. The current M1.1 workspace-config implementation predates this accepted target.
- **Building-block registry** — The resolver for addressable Tools, Instructions, Phases and Flows. In v0 it safely catalogs one-block YAML entries under a configured root, then retains and resolves only the selected Flow's transitive definition closure, once per unique definition.
- **Canonical serialization** — Deterministic UTF-8 JSON of the parsed, semantically validated and registry-resolved selected definition closure; equivalent closures serialize to the same bytes for review, audit and golden tests.
- **Tool** — A Phase-scoped capability with an exact command identity (predefined or own script) and declared parameter, path and network boundaries. Referencing it only makes it available; the provider decides whether and how often to request it.
- **Policy artifact** — The canonical JSON output produced by `core-policy` to show Tool-scoped capabilities for a target enforcement backend, including deterministic object-key and array ordering. M1 evaluates compatible decisions in process but provides no OS security boundary.
- **Predefined-command registry** — The trusted id-to-executable map used by predefined-command Tools. A script names a `command_id`; Flow Agent resolves it to one executable identity and combines it with the script's literal base `argv` without PATH lookup or shell parsing.
- **Predefined-command Tool** — A Tool that names a fixed command id resolved through the predefined-command registry. M1 fixtures emulate its result; M1.1 direct-execs the resolved command under bounded runtime controls.
- **Allowed parameter** — A reviewed parameter spec for a Tool: exact name, typed value shape, required flag and type-specific constraints such as enum values, string pattern/length or integer range. Unknown parameters and values that fail validation are denied before tool launch.
- **Own-script Tool** — A Tool whose reviewed inline `script_body` uses the fixed v0 `posix-sh` contract. M1 interprets only its bounded fixture subset; real runner and isolation stages are defined in the [Flow Agent V-Spec](docs/concept/V-Spec_FlowAgent.html) and [`SECURITY.md`](SECURITY.md).
- **Instruction** — A modular prompt primitive with typed `{{name}}` parameters. A leaf Phase references Instructions and binds their declared values from its complete typed input; an explicitly selected source filename has no authority or naming semantics.
- **Phase** — The recursive workflow unit. A leaf exposes Instructions and Tools to one provider loop and returns a typed result. A composite runs ordered child Phases without its own provider turn and selects one child's result. Either may use a bounded declarative loop.
- **Transition** — An ordered inline forward-sibling rule owned by a Flow or composite Phase. It selects a later sibling only when an exact typed equality predicate matches the successful source result; otherwise execution falls through.
- **Flow** — A fully configured deterministic orchestration of ordered Phases and optional subflows. Whole successful results pass forward by default; a Flow is itself a building block.
- **Subflow** — A flow used inside another flow.
- **Flow execution plan** — The finite typed, deterministic and side-effect-free description of resolved Flow invocations, lifecycle boundaries, context inputs and Tool intents consumed by runtime apply.
- **Execution request** — One versioned, bounded request from Flow Agent to an Executor for exactly one validated Tool invocation. It carries the executable identity, argument vector, working directory, explicitly allowed environment, limits and compiled policy; it grants no authority beyond that policy.
- **Executor** — A Flow Agent execution component that understands the Execution request/result protocol, validates and translates the compiled policy, manages one Tool process lifecycle and returns bounded results and enforcement evidence. It orchestrates isolation but does not have to implement the operating-system isolation primitive itself.
- **Default Sandbox Executor** — The official Flow Agent Executor installed by the standard M1.2 installation path. It maps Flow policy to the supported Linux or macOS Sandbox backend and fails closed when that boundary is unavailable or cannot be proven ready.
- **Custom Executor** — An administrator-selected implementation of the Executor protocol. Flow Agent documents the protocol and reports integration failures, but does not certify or guarantee a third party's compatibility, security or operation.
- **Meta-Harness agent executor** — Meta-Harness's host-local component for launching and supervising whole CLI agent processes. It is not a Flow Agent Executor and never selects, configures or manages a Flow Tool Sandbox.
- **Fixture executor** — The M1 in-process executor enabled only by the canonical fixture profile; it returns deterministic model/Tool doubles and is not a provider, subprocess runner or POSIX shell.
- **Compatibility probe** — A non-authoritative Executor handshake and self-test that can detect known protocol, platform and setup mismatches. Passing it is useful diagnostics, never a compatibility or security guarantee.
- **Policy emulation** — M1's deterministic in-process evaluation of modeled policy decisions. It proves runtime correctness contracts, not OS isolation.
- **Sandbox** — The restricted working environment in which an untrusted Tool runs. The term describes the security boundary, not the component that interprets Flow Agent rules.
- **Sandbox backend** — The operating-system, container or virtual-machine mechanism that constructs and enforces a Sandbox. Bubblewrap plus seccomp and macOS Seatbelt are M1.2 backends; they do not understand Building Blocks by themselves.
- **Container** — A Sandbox form that isolates processes while sharing the host kernel. It can be an Executor backend, but is neither an Executor nor automatically a sufficient security boundary.
- **Virtual machine (VM)** — An isolated guest system with its own kernel. A VM or micro-VM can be an Executor backend; higher isolation usually costs more startup time and operational complexity.
- **OS isolation** — Enforceable filesystem, process and network restrictions applied by a Sandbox backend to each real Tool process and its descendants. Provider traffic remains Flow Agent traffic outside the Tool Sandbox.

## Flow Agent runtime surfaces

- **flow-value-v0** — The closed, explicitly tagged and provider-neutral M1.1 value contract shared by root input, Phase input/results, Instruction parameters, Tool parameters and Tool outputs. It permits no implicit coercion or floating-point values; large or binary values use immutable session-object references (ADR-0092, ADR-0098).
- **flow-run-input-v0** — The versioned run-input document containing one complete `flow-value-v0` value for the selected root Flow; its canonical contract is in `PROTOCOL.md`.
- **flow-tool-result-v0** — The fixed Tool-result envelope for status, stdout and stderr flow values, with exit code only when observed; its canonical contract is in `PROTOCOL.md`.
- **flow-m11-budget-v0** — Optimized Ubuntu JSONL evidence schema for the finite ADR-0107 M1.1 budget matrix; it records exact workloads, raw samples, aggregates and gate outcomes without adjusting measured RSS.
- **Conversation** — The complete user-facing work history: one append-only tree of navigable entries and all Runs that belong to it. Continuing from an earlier entry creates a branch without deleting descendants.
- **Portable continuation** — A verified, continuation-eligible terminal Conversation checkpoint continued on another device or Runtime binding as a new child Run. It rebinds authority and capabilities, never replays prior effects, and is distinct from exact interrupted-Run recovery.
- **Conversation entry** — One versioned history node that links to its parent and to a committed point in one owned linear Run. New productive `flow-conversation-entry-v1` nodes also bind the terminal recovery snapshot used for compact continuation; migrated legacy roots remain v0.
- **Run** — One complete execution of a Flow inside a Conversation, from Flow start through its Phases to completion or failure. Each continuation or branch starts a new Run. Its technical Run ID is stored as lowercase path-safe `run_session_id`; events use it as `session_id`, and exact replay, Tail or interrupted-Run recovery address it together with its owning `conversation_id`.
- **Transcript** — The messages and runtime events reachable on a selected conversation branch; the complete tree retains every branch.
- **Durable session history** — The append-only conversation tree plus every owned linear run and referenced immutable source artifact. Provider-context optimization and branch navigation never delete it or roll back external effects.
- **Run Log** — The canonical versioned append-only `run-log.jsonl` for one Run. Its first record preserves the definition identity and hashes required for Resume; later records durably track provider and Tool dispatch attempts, and per-Tool Run Logs project only Tool records rather than using separate files.
- **Recovery snapshot** — One bounded, versioned `recovery.jsonl` prefix for a productive Run. Its synchronized records preserve enough deterministic orchestration state and durable-result references to resume the exact Run without repeating completed provider or Tool calls; its terminal record also preserves compact cumulative state for a child Run.
- **Uncertain attempt** — A durable provider or Tool intent without a committed terminal result. Flow Agent never redispatches it automatically; `PROTOCOL.md` owns reconciliation behavior.
- **Run bundle** — One Run's event/context/recovery segments, Run Log including its definition metadata, and immutable objects.
- **Conversation bundle** — One conversation history plus all owned Run bundles.
- **Host-local session ownership leases** — Exclusive operating-system file leases in the private user-global Workspace store. `PROTOCOL.md` defines their ownership and acquisition contract. Process exit releases leases; none is durable or transferable across hosts.
- **Session marker** — The persistent `session.lock` observability leaf inside a run directory. It is not ownership authority and cannot grant or revoke the lease.
- **Reservation orphan** — An empty run artifact retained and inventoried when rollback cannot prove identity-bound deletion. Its cleanup failure remains visible, and compare-then-unlink never removes it.
- **Event segment** — One append-only canonical JSONL file within a run bundle; rotation bounds individual I/O without starting a new run or resetting sequence/budgets.
- **Resolved flow state** — The current Flow invocation and Phase execution stack, active Instructions and available Tools, typed input/result and runtime state exposed by defined interfaces.
- **Provider context** — The deterministic, bounded projection compiled from resolved flow state and narrowly selected durable history for one model turn; not the full transcript.
- **Context profile** — A versioned deterministic contract for provider-context ordering, budgeting, tokenization/estimation, projections, hashing and cache boundaries; M1 exposes only `flow-context-v0`.
- **Context manifest** — The reproducible per-provider-turn record of the context profile, budget inputs, ordered included/projected sources, omissions, cache boundaries, token estimate and final context hash.
- **Runtime event** — One normalized event Flow Agent emits over its public contract (see `PROTOCOL.md`); the same events feed JSONL mode, future RPC mode and the session store.
- **JSONL event stream** — Flow Agent's headless mode that streams newline-delimited JSON runtime events to stdout for automation/CI/consumers.
- **RPC mode** — Flow Agent's designed-for bidirectional stdin/stdout control mode. ADR-0055 selects the initial command/request shape; runtime events remain the public event contract.
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
- **Network allow entry** — A typed CIDR/IP egress rule with transport and port; the only v0 way to declare network access. M1 Linux-target deterministic policy rejects non-empty allowlists until D-046 selects a positive egress backend.
- **Protected path** — A path pattern denied by default even inside a declared root unless a flow explicitly grants it.
- **Protected-path grant** — A tool-scoped exception that removes the protected-path deny only when the path is still inside that tool's declared read/write scope.
- **Smoke-flow** — The smallest golden flow: one phase, one tool and one instruction, used as the first localizable gate.
- **Hello-flow** — The showcase golden flow that exercises multiple Phases, scoped Instructions/Tools and subflow reuse.
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
