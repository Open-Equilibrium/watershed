# Security

Cross-cutting security model for all tools. Do not re-decide these per tool.

Product targets, available capabilities and native release requirements are canonical in [PLATFORMS.md](PLATFORMS.md). A release target never enables an unproven execution boundary.

## Accepted Flow Agent security target

**ADR-0166–ADR-0175 define the integrated replacement; native verification is pending.** This section owns the Flow Agent security contract. The checked-in runtime, invocation policy and Executor wire now use trusted Tools with mandatory native self-protection on Linux and macOS. CI and installation acceptance are being migrated to that contract. Integration and passing Windows shared tests do not establish native protection or release readiness; [TESTING.md](TESTING.md#m12-transition-and-executor-evidence) owns the previous baseline, red replacement run and outstanding native proof. Mac Executor launch is currently blocked on [D-069](docs/decisions/open-decisions.html#d-069); its proposed installation-trust change is not an accepted guarantee.

### Guarantees and owners

| Boundary | Flow Agent must enforce | Engineer / Tool responsibility |
|---|---|---|
| Model requests | Only Tools available in the active Flow/Phase, valid declared parameters, legal transitions and existing execution bounds may proceed. Model text and Tool output cannot grant authority. | Configure a suitable Flow and Tools. An allowed request is not proof that the requested action is sensible. |
| Tool effects | No general host filesystem, network or hostile-code containment promise. Validate the invocation, not every instruction subsequently executed by its program. | Trust the complete executable chain, libraries, helpers, services and any project code run by build/test commands. Implement promised path/action limits for adversarial inputs, not only ordinary use. A descriptive Building Block field is not OS enforcement. |
| Flow-owned files | A mandatory native boundary blocks direct Tool/child writes, deletion and replacement of this installation's protected Flow configuration, registry, runtime/credential stores and installed program objects. Tools cannot choose the protected set or disable it. Missing protection fails before Tool launch. | Do not treat this as whole-host or cross-installation isolation. Privileged actors, controller/OS compromise and effects delegated to independent unconfined services are excluded. File/object discovery and inherited protection require native proof. |
| Configuration changes | The first Flow Agent release provides no Tool-initiated configuration-change path. Direct protected writes remain blocked; no permission prompt or automatic retry lifts the block. | Administrators change configuration manually outside Tool execution. Later proposals belong to the roadmap, not first-release capabilities. |
| Authority integrity | A Tool cannot grant itself authority, widen its own permission, replace enforcement code or silently change the authority of the current Run. | Administrators remain responsible for trusted installation and deliberate out-of-band changes. |
| Effects and recovery | Retain durable intent before effects, bounded I/O and waiting, cancellation of further dispatch, truthful terminal evidence and no automatic replay of uncertain effects. | A successful command is not proof of a correct result. Tool effects on external systems need their own correctness and recovery design. Unknown descendant cleanup must not be reported as complete. |
| Credentials | Never automatically forward Flow-owned provider credentials through Tool environments, requests, results or logs. Retain the provider-authentication contract below. | Write protection alone is not read isolation or secret classification. Deliberately admitted inputs and Tool-returned data may contain secrets. Do not infer credential confidentiality against arbitrary host-authority code. |

Flow's own configuration, context, provider and storage operations are internal runtime responsibilities, not model-selected Tools. The promise is **controlled invocations and workflows with trusted Tool implementations**, not that every runtime file access appears as a Building Block command.

### Native self-protection and its limits

ADR-0171, corrected by the maintainer on 2026-09-08, excludes only Workspace-local `AGENTS.md` instruction inputs from Flow-owned write protection. They remain editable when the configured Tool permits it. The global `AGENTS.md` inside `FLOW_AGENT_HOME` stays protected with that home; a Workspace pointing at the global home does not turn it into an editable local exception. Neither instruction source grants technical authority, and editable local instructions can still influence model behavior. The filename does not exempt an alias of a protected object. Native acceptance must distinguish editable local instructions from the protected global file and enforce ADR-0169 alias rules.

ADR-0172 permits deliberate installation/home placement inside a Tool-writable directory after a clear warning identifying the retained protected objects and unavailable ancestor moves/deletion. Location approval never grants direct Tool writes to Flow-owned files. Narrower native restrictions remain mandatory; unsafe aliases, unverifiable inventories and unavailable enforcement still prevent launch rather than becoming warning-only exceptions.

Tools and newly started helpers must inherit the direct-write restriction, including when a helper is compromised. Ordinary helper creation must not escape it. A trusted Tool that asks an already-running editor, automation service or remote system to act delegates to that system's separate authority; such effects are not brought inside the boundary by the request. The Engineer must trust that integration. The narrow protection does not certify malicious Tools as safe, their outputs as truthful or project files as confidential.

ADR-0174 limits the first-release protected inventory to this installation's selected global Flow home, its external Flow-owned platform stores and its installed program objects. Concurrent Runs sharing that home share its protection; there is no automatic discovery or mutual protection of other independent homes. The exact object classes are listed in the [native protection design](docs/concept/flow-agent-native-protection-proposal.md#complete-protected-set). Additional-home protection is only a [later discussion](PLAN.md#later-flow-agent-roadmap), not an approved feature.

ADR-0169 accepts the file-layout restrictions: every hardlink name of a protected file must remain inside the protected set; internal publication aliases remain possible, but an outside alias or unverifiable inventory prevents Tool startup with an explanation, never automatic repair. Tools cannot remove or move ancestors containing protected Flow objects. Ordinary unrelated project-file operations remain outside that restriction. The maintainer also reconfirmed the independent-service exclusion above. The Mac mechanism follows ADR-0172.

ADR-0170 accepts the first-release compatibility cost of denying Tools direct terminal-device access to protect human review. ADR-0173 defers the configuration review feature, not this separately accepted restriction. Some interactive programs may be unavailable; a question over ordinary stdin is not inherently terminal-device access. This accepts the restriction, not a blanket claim that all interactive programs fail or that every noninteractive program works. Do not weaken protection or forward credentials as a workaround. A private interactive Tool terminal remains a separate design decision; mechanism approval supplies no missing native integration evidence.

The native Linux x86_64/macOS ARM64 backends replace general exact-mount/runtime-read-profile policy, Tool deny-all networking, process/thread ceilings and absolute hostile-descendant cleanup guarantees. Linux binds the host root and overlays targeted read-only protection, read-only `/proc`, private `/dev` and seccomp; Mac uses narrow Seatbelt rules. Networking remains available within the host's authority. Neither backend uses per-Tool runtime profiles or systemd/cgroup capacity enforcement. Bounded output, deadlines and cancellation remain required. No automatic unprotected fallback or Linux VM product path on Mac is permitted.

ADR-0167 authorized the bounded native feasibility evaluation. ADR-0172 selects Seatbelt through `/usr/bin/sandbox-exec` for macOS ARM64, accepting the deprecated-interface and OS-update maintenance risk. The integrated backend applies narrow Flow-owned direct-write and terminal restrictions; it is not a general filesystem/network permission product. Checked admission, inherited protection and fail-before-launch behavior require native revalidation, including after OS updates. Supported-version, complete runtime and installation/signing evidence remain release gates in [TESTING.md](TESTING.md).

An additional external sandbox may further restrict Flow and its Tools. It must permit the required runtime storage, provider access and native protection mechanism. Nested compatibility is deployment-specific and requires real tests; failed readiness cannot disable Flow's mandatory protection. Outer containment can strengthen a deployment but never changes Flow's own advertised guarantee or certifies an unsupported platform.

Concurrent Runs do not imply independent OS security domains. The existing user-global configuration authority remains shared, while Run ownership and storage follow `PROTOCOL.md`. Correct Tools may still race on shared project files or exhaust shared memory, disk, CPU or GPU. Coordination and resource planning remain deployment responsibilities; a particular number of parallel agents is not a safety or performance guarantee.

### Configuration and migration boundary

**First Flow Agent release (ADR-0173): manual configuration administration only.** Implement no Tool configuration-proposal protocol, per-Building-Block `ask`/scoped `allow` configuration policy or approval UI. Direct Tool/child writes to protected Flow objects remain blocked, without converting OS errors into prompts. Existing manual authoring, configuration validation and safe publication remain required, as do Flow's internal runtime/storage writes. This deferral neither freezes runtime state nor grants Tools access to trusted administration paths.

The unreleased Flow definitions, schema, compiled policy, protocol, goldens and tests migrate together without compatibility aliases. Removed isolation requirements are rejected before effects, never accepted as ignored security settings. Replacement behavior requires meaningful red-first tests and native evidence on both release targets. Standard Tool scope and trust UX remain [D-066](docs/decisions/open-decisions.html#d-066); the later marketplace remains [D-067](docs/decisions/open-decisions.html#d-067), not a certification promise.

The [later roadmap](PLAN.md#later-flow-agent-roadmap) retains the deferred configuration feature and prior discussion context, without a future protocol, UI specification or acceptance matrix here.

The [security architecture and Mermaid case matrix](docs/concept/flow-agent-executor-architecture.md) explain these boundaries without defining a second policy. Meta-Harness, Liquid, download trust and development/publication safeguards are unchanged.

## Reporting a vulnerability

Report suspected vulnerabilities privately to **b-weber@gmx.at** — please do not open public issues for security problems. Include reproduction steps and affected files/components where possible. Reports are handled on a **best-effort basis**: this project gives **no guarantees** of response time, fixes, or any warranty of any kind; the software is provided "as is" (see `LICENSE`, AGPL-3.0-only §15–16). Coordinated disclosure is appreciated.

## Trust model

Flow Agent's [contract above](#accepted-flow-agent-security-target) governs its invocation and native self-protection boundary. Other products retain their own capability and mutation contracts.

Watershed combines structured Flows, validated invocations, normalized events, policy gates and metric feedback with Liquid's permissioned workspace mutations and action history, and Meta-Harness's audited configuration control. Flow Agent trusts its admitted Tool code; Liquid and Meta-Harness retain their distinct boundaries below. AGPL/free-software transparency lets users inspect, self-host, fork and verify core behavior; it is not a substitute for enforcement.

## Principle: scripts define; enforcement must match the claim

Scripts define available Tools, executable/argument contracts, declared parameters and workflow structure. The harness compiles an invocation policy per Flow and validates each request before effects. The Executor adds controller-selected native protection of Flow-owned objects. Fixture evaluation does not establish OS protection, and command allowlisting does not constrain every effect of trusted program code.

This paragraph governs Flow Agent scripts. Liquid Apps use the parallel principle defined below: App manifests declare capabilities, and the App Runtime plus Role and capability checks enforce them.

### M1.2 Tool execution trust boundary

1. Before durable productive Run reservation, Flow Agent admits the installed Executor and protected inventory and probes readiness. For each Tool call it validates the invocation and synchronizes durable intent. The same one-shot Executor returns `Ready`; Flow Agent commits `tool.started` and only then sends matching `Start`.
2. The Executor rejects unsupported requests before launch, waits after `Ready`, and establishes the native restriction only after `Start`. Before the first Tool instruction it checks protected identities and removes unintended inherited handles. Its receipt binds the exact resolved policy and reports `self_protection_active`.
3. The native boundary denies direct Tool/child modification of the admitted Flow objects and their protected ancestry. Tool code and its complete execution/delegation chain remain Engineer-owned trust decisions. Bounded Tool-root supervision does not prove that all hostile descendants ended.

Provider connections remain Flow-owned and provider credentials are never forwarded to the Executor. Ordinary Tool networking is allowed. The standard installation includes the Default Executor; explicit Custom selection joins the installation's trusted computing base and still requires the same protocol and protection contract. Under ADR-0175, only explicit Custom selection permits an absent official sibling; any existing sibling remains required to pass admission and be protected.

Building Blocks, model output and Workspace-local files cannot select or replace an Executor. Missing or failed admission/readiness prevents launch without fallback. Custom evidence is structurally checked, not certified as truthful. Fixture execution remains deterministic emulation. [PROTOCOL.md](PROTOCOL.md#m12-executor-protocol-adr-0146-adr-0160-adr-0161-adr-0162) owns the wire and backend mechanics; [TESTING.md](TESTING.md#m12-transition-and-executor-evidence) owns the pending native proof.

Because scripts are human-reviewable security/capability artifacts, they pass through one private `core-script` Safe-YAML parser into one unambiguous model (ADR-0031, ADR-0061). It accepts one YAML 1.2 document and rejects duplicate or merge keys, anchors, aliases, explicit tags, nulls, unknown fields and configured resource-budget violations; there is no fallback parser. The checked-in JSON Schema files document the intended shape, existing semantic and registry validation remains authoritative, and the Flow Agent V-Spec defines canonical bytes.

Installation distributes versioned download bundles containing prebuilt artifacts and the installer, rather than `.deb` packages, and makes ordinary dependencies explicit. Missing native mechanisms or incompatible host security policy must produce an actionable failure, not automatic kernel upgrades, AppArmor/sysctl changes or service activation. Administrators decide host-wide changes separately. The replacement has no systemd user-manager or delegated-cgroup prerequisite; product execution remains unprivileged.

The first Flow Agent release trusts the official [GitHub Releases channel](https://github.com/Open-Equilibrium/watershed/releases) over verified HTTPS, with checksums for the final download bundles (ADR-0165). The initial installer is trusted through that channel and the operating system/browser's HTTPS verification, not through self-verification after launch. Matching checksums detect bytes that differ from the published package; they do not independently authenticate a compromised publication channel. An independent build-provenance verifier or separately managed release-signing key is not required for this download contract. Mac signing/notarization requirements of the selected native mechanism remain separate.

Native installer and release-artifact acceptance are being migrated; completed download-distribution proof is still outstanding. Before the first Flow Agent release, prove clean native installation without compilation or extra developer tools: verify version, platform and final bundle checksum before executing its payload; reject failed HTTPS verification, missing verification data, altered or wrong-target packages; cover every shipped executable and installer byte after any Mac signing/notarization/stapling. Retain the [create-only installer boundary](README.md#developertest-installation-on-linux-or-macos): use the intended unused prefix, reject unsafe paths, expose privileges/dependencies and recover only this attempt's own files on failure. This adds no in-place upgrade or updater. Development/publication safeguards remain [D-065](docs/decisions/open-decisions.html#d-065); ordinary branch CI does not prove they are active.

Registry access starts from one opened capability for the Global Flow home. Loading opens every registry directory and YAML leaf without following links. M1.1 authoring must open or create each component relative to its already-open parent without following symbolic links, use exclusive no-replace creation, and verify the opened object's type and identity before descent; it never follows a successful path check with an ambient path reopen. This private boundary applies on Flow Agent's native targets (ADR-0063, ADR-0064, ADR-0163).

Flow Agent configuration, registry and runtime state live in the private user-global home defined by `PROTOCOL.md`. Workspace `.flow/config.yaml`, Workspace registries and other ambient project configuration have no technical authority and are never probed, merged or used as fallback. Missing, invalid, inaccessible, unsafe or conflicting global state fails before Run/session mutation. Optional global-home and harness-start Workspace `AGENTS.md` files remain a separate bounded instruction/context channel; their content cannot configure providers, models, registries, Runtime bindings, credentials or resource policy. The runtime store uses owner-only permissions and fails closed when that boundary cannot be established. The `session.lock` leaf is only a persistent observable marker. Marker mutation cannot grant or revoke ownership, and process exit releases operating-system leases automatically. This is concurrency control, not OS isolation: an unconfined peer with the same OS identity can still corrupt runtime state or tamper with leases, and cross-host or durable ownership remains post-M1. Distinct canonical Workspace paths deliberately use distinct runtime stores, never distinct Flow configuration authorities.

Complete-history validation creates only a private per-command scratch index in that Workspace store; it is never conversation state. Unsafe scratch or insufficient admitted space fails before provider or Tool effects. Success, error and later crash recovery remove only the opened index identity and its verified files; replacement, links, unexpected names or mismatched identities fail closed without deleting foreign bytes. Cleanup assumes cooperative writers under the same OS identity; hostile same-identity replacement is outside the M1.1 guarantee.

Conversation storage maintenance and internal failed-creation rollback are defined in [`PROTOCOL.md`](PROTOCOL.md#local-run-storage-and-m11-conversation-trees).

- **Command allowlisting limits names, not effects.** Interpreters, build scripts and hooks can execute further code. A narrow Tool can enforce a finite input/path contract, but command-name approval cannot confine arbitrary executable effects; safe argument handling and the entire invoked code chain remain essential.
- **Agent intent is untrusted (prompt injection / confused-deputy).** Combining untrusted content, private data and an exfiltration path creates a risk that prompt hardening alone cannot eliminate. Flow controls invocation authority; the Engineer must select Tool implementations and deployment boundaries appropriate to the admitted data and effects.

`flow-context-v0` always includes base runtime/security instructions in mandatory Tier 0 and fails before provider contact if they do not fit (ADR-0058). This protects instruction integrity and provider-cache consistency, but prompt text remains defense in depth. M1 policy evaluation is a deterministic correctness boundary, not process isolation; native self-protection supplies only the direct-write boundary defined above for real Tool processes.

## Provider authentication

The exact Flow-owned OAuth wire, cache, refresh and local-logout lifecycle is canonical in [`PROTOCOL.md`](PROTOCOL.md#m11-codex-subscription-provider). Flow Agent owns that current-user-protected cache, never imports another client's credentials, and never inserts its own provider credentials, account ids or authentication bodies into events, conversation history, Run Logs, diagnostics, exports or Tool environments. A definitive provider failure may persist and display the provider's direct message under the bounded contract in `PROTOCOL.md`; provider-supplied content remains the provider's responsibility. ADR-0107 selects the provider parser and temporal bounds; their evidence in the [M1.1 budget matrix](flow-agent/benchmarks/M1_1_BUDGETS.md) must pass before productive behavior is enabled.

Agentic Engineers configure Building Blocks and trust their complete Tool implementations; other users may run those predefined Flows without widening their authority. Ordinary assigned Tool execution needs no confirmation. Provider authentication remains Flow-owned and is never forwarded in an Execution request. Before provider or Tool dispatch, Flow Agent synchronizes durable intent. For Tools, `Ready` grants no launch authority and matching `Start` is the effect boundary: pre-Start EOF, cancellation or commit failure cannot launch a Tool. Missing terminal evidence after `Start` is uncertain and never retries automatically. The exact reconciliation command is defined in `PROTOCOL.md`.

## Accepted post-M1.1 target: local inference and portable continuation

This section defines unimplemented requirements; current M1.1 has neither Runtime bindings nor Portable continuation.

A future local model endpoint changes availability and data movement, not trust. Model output remains untrusted; the same typed values, context bounds, Tool validation, durable intent and the accepted Tool security boundary must apply whether inference is local or remote. The local inference process is executable host code outside the Tool's native protection boundary; D-061 must decide whether it joins the trusted computing base or runs behind an enforceable identity/filesystem/network/secret boundary. Artifact signatures prove provenance, not confinement, and no productive offline-isolation claim exists until that boundary is implemented and tested. A Runtime binding may name only a typed credential reference bound to the selected provider and endpoint audience; Flow Agent must reject any mismatch before resolving credential material. Credential material stays in the Flow-owned store and retains its locking, refresh, protection and redaction contract. Provider output must not select the binding's endpoint, model/runtime artifacts, credential reference or resource policy, and a binding must not become Conversation authority or silently widen a Flow's capabilities. Offline-after-provisioning behavior and local model/runtime supply-chain evidence remain [D-059](docs/decisions/open-decisions.html#d-059).

Portable continuation must transfer verified context, not authority. Standalone Flow Agent must authenticate the destination's local OS actor, revalidate the selected Flow and capability/policy envelope, admit required resources and create a new child Run before effects. Integrations must independently reauthorize their own resources; accessing Liquid additionally requires the destination's effective Role and session grant. The archive must exclude credential-store records, Runtime-binding credential references, approvals and host-local leases and grants no implicit right to a Workspace or external system. Conversation content is still sensitive and may contain secrets previously supplied or exposed by users, providers or Tools; D-058 must define archive access, confidentiality and transfer protection and cannot promise secret-free bytes. Content hashes alone do not authenticate provenance: D-062 must bind the canonical root to an authenticated identity or classify the import as unauthenticated, non-executable evidence. Completed provider and Tool effects must not be redispatched; uncertain attempts retain the existing fail-closed reconciliation boundary. Direct private-store copying is not an import mechanism. Archive and branch rules remain [D-058](docs/decisions/open-decisions.html#d-058).

An offline device cannot learn a remote revocation while disconnected. No design may claim otherwise. Prior approvals must never transfer with a Conversation. Whether a destination may issue narrowly scoped local approvals while offline, and how expiry, one-time use and later revocation interact, remains [D-060](docs/decisions/open-decisions.html#d-060). Until that contract exists, imported sessions receive no portable approval authority.

## MVP VCS boundary

Flow Agent runs inside normal Git projects in the MVP, but it does not own project history and does not implement project VCS behavior. Auditability comes from deterministic Flow state, structured logs, protocol events, manual configuration records and policy decisions; it does not come from an OS sandbox. Host Git may run only as an explicitly declared Tool and receives the same execution boundary as every other Tool; its implementation and invoked project code remain part of the Engineer's trust decision under ADR-0166.

## Enforcement (per flow)

Flow validation, native self-protection and durable lifecycle are the separate responsibilities defined [above](#m12-tool-execution-trust-boundary). [PROTOCOL.md](PROTOCOL.md#m12-executor-protocol-adr-0146-adr-0160-adr-0161-adr-0162) owns the exact resolved policy, receipt and Linux/Mac mechanics; do not infer general filesystem, network or resource containment from an enforcement receipt. Optional external containment follows the deployment limits in [native self-protection](#native-self-protection-and-its-limits).

## Meta-Agent configuration writes

A Meta-Agent may reconfigure underlying agents, **policy-gated**: low-risk changes apply only through the configured review/audit flow; sensitive changes (permissions, tools, network, schedules, external credentials) require human approval. Every change is recorded with who/what/when, is monitorable and is revertible according to the chosen config-storage model. The human always knows what changed.

The same gate applies to **all Meta-Harness control surfaces** — its CLI, API/service and BYOA/external command surface. Meta-Harness runs headlessly (without Liquid), so the policy/audit gate, not a UI confirmation dialog, is the boundary: sensitive commands from any client must be authorized and audited identically.

Execution ownership is host-local. A Meta-Harness agent executor may control only whole CLI agent processes created or adopted on its own host under an explicit local identity; it rejects cross-host process claims. This process supervisor is not a Flow Executor and has no authority to select, configure or manage Flow Tool Sandboxes. Exposing the API to another device requires authenticated, integrity-protected transport and does not expand agent-executor authority. Liquid must route live commands to the instance that owns the addressed session/configuration and must not treat cached state as controllable while that instance is unreachable.

## Liquid workspace access & external-agent edits

Liquid is a standalone workspace product that external agents and tools can read and edit through its workspace CLI/API. That access is permissioned and auditable:

- CLI/API access requires an authenticated identity and an assigned allow-only **Role**. Unlisted resources and actions are denied by default; explicit deny/blacklist rules are deferred.
- Roles may be assigned to users, groups, agent profiles, sessions and Automations and may allow discovery, proposal, execution, approval or management over named Workspaces, Pages, Blocks, Sources, App actions and Meta-Harness projections.
- In the M3 MVP, the local confidentiality boundary is the Workspace: authorizing a device to replicate it makes every plaintext replica byte accessible to the device owner and same-identity local processes. Resource-scoped discovery/read permissions remain enforced by Liquid but cannot hide those bytes. This does not grant write, execution or administrative authority.
- Effective authority is the intersection of the Role and narrower system, App, session, provider and execution-host boundaries. No layer can grant a capability another boundary denies.
- Every workspace write — from the UI, Liquid AI, the CLI/API or an external agent — goes through one **permissioned mutation pipeline** and is recorded in Liquid's **action history**; there are no hidden writes that bypass it.
- Sync applies received actions through that same pipeline. Sync credentials authorize Workspace exchange only; they do not authorize Meta-Harness control. Interrupted or untrusted sync never disables access to the local replica.
- A headless Liquid replica is a separate execution boundary. It receives only Workspaces explicitly enabled for that replica, then enforces the same Roles and mutation pipeline as a UI replica.
- External-agent writes are **attributed** (actor/origin) and **revertible**; sensitive changes require approval, and a proposed diff can be reviewed before apply.
- Secrets/credentials stored in workspace data require special handling.
- App execution, external MCP calls and external-agent edits are separate risk classes and keep separate capability grants.

This is Liquid's **workspace** action history (over Liquid's own data), not a project-code VCS. Detail: [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html).

### Liquid Apps, Block packages and MCP

- App code runs locally in the restricted App Runtime. The first target is isolated JavaScript/TypeScript with declarative UI, explicit capabilities, CPU/memory/time limits, no ambient filesystem/process/environment access and deny-by-default network access. WASM is a later runtime target.
- App state changes and App-driven workspace writes use the mutation pipeline. App code cannot edit another Block merely because a View is nearby; a Connection plus an effective permission is required.
- An App action is capability-scoped and may be invoked by UI, Connection, Automation or agent only when the caller and App both allow it.
- External MCP servers remain outside the App Runtime. Liquid's MCP adapter is the client boundary, validates declared inputs/outputs and maps only granted capabilities to typed App actions. MCP connectivity never grants broader Workspace access.
- Block Registry packages are signed, versioned, sandboxed and capability-scoped. Initial support loads no arbitrary third-party native code; package update and migration are explicit, reviewable actions.

## Plugins & supply chain

- Post-M1 plugins run as **Wasmtime** modules: capability-scoped, sandboxed, with explicit grants and resource limits.
- Dependency hygiene: committed lockfiles, exact pins and minimal dependencies; CI runs `cargo audit` (RustSec advisories), `cargo deny` (license/bans/sources/advisory policy via `deny.toml`) and `pnpm audit`. `cargo vet` remains an optional later addition. Rust reduces but does not eliminate supply-chain risk (`build.rs`/proc-macros run at build time); future isolated runtimes must limit blast radius regardless of language.

## M0/M1 policy-emulation scope

Fixture execution checks modeled invocation decisions in process; it never proves native protection. The retained `agent-negative` workloads exercise forbidden-write, network/DNS, environment, out-of-phase, link and interpreter failure lifecycles through explicit fixture labels. Those historical scenario names do not grant or assert productive filesystem/network containment. Own-script fixture output remains create-only: an existing leaf rejects before runtime or temporary-file mutation (ADR-0087); output replacement remains [D-056](docs/decisions/open-decisions.html#d-056).

### M0 policy artifact contract

`core-policy` emits canonical invocation-policy JSON for review and deterministic tests. Serialization follows [PROTOCOL.md](PROTOCOL.md#canonical-protocol-json-serialization-v0), with a final LF. It has no OS target, filesystem/network policy, runtime-read profile or process/thread capacity field; legacy isolation fields are rejected by the closed schema.

| Field | Contract |
|---|---|
| `policy_version` | Fixed string `"0"`. |
| `source_flow_definition_id` | Resolved Flow definition id. |
| `commands` | Tool invocation records; canonical shape and validation live in [CommandPolicy](core/core-policy/src/artifact/command.rs). |
| `phase_scope` | `{phase_id, tool_ids}` records defining available Tools. |
| `runtime_limits` | `{headless, timeout_ms}`; Boolean headless flag and integer deadline. |

Each command records `tool_id`, `tool_kind`, `command_id`, `executable`, literal `argv`, `allowed_parameters`, `environment` and, only for own-script Tools, `script_runtime: "posix-sh"`. Predefined commands resolve through the trusted registry; own-script commands use `script:<tool-id>` and the fixed POSIX-shell runner. Productive invocation uses the verified executable and literal arguments without `PATH` lookup, shell parsing, expansion or globbing by Flow. Parsing the reviewed own-script body remains part of that Tool's trusted semantics. U+0000 in execution-vector strings rejects before spawn.

Parameter contracts define exact names, kinds, required values and their applicable enum/string/path/integer constraints. Unknown parameters, extra positional arguments and invalid values reject before launch; their rendering is canonical in [PROTOCOL.md](PROTOCOL.md#m11-runtime-values-adr-0092-adr-0098).

`environment` is `{default: "clear", allow}`, with exact Engineer-selected host variable names matching `^[A-Z_][A-Z0-9_]{0,63}$`. The current script compiler emits `allow: []`. Artifacts/events contain no environment values; the resolved request carries only explicitly admitted values. Flow performs no automatic secret classification of configured values, and never forwards its own provider credentials.

Commands sort by `tool_id`; allowed parameters by `name`, enum values and environment names lexicographically; Phase scopes by `phase_id` and their Tool ids lexicographically. Literal `argv` order is preserved. New arrays must preserve schema order or define a sort key before fixture publication.
