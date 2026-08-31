# Security

Cross-cutting security model for all tools. Do not re-decide these per tool.

## Reporting a vulnerability

Report suspected vulnerabilities privately to **b-weber@gmx.at** — please do not open public issues for security problems. Include reproduction steps and affected files/components where possible. Reports are handled on a **best-effort basis**: this project gives **no guarantees** of response time, fixes, or any warranty of any kind; the software is provided "as is" (see `LICENSE`, AGPL-3.0-only §15–16). Coordinated disclosure is appreciated.

## Trust model

Watershed's defensible trust model is the combination across its three layers: structured flows + scoped runtime capabilities + normalized events/transcripts + policy gates + metric feedback + permissioned workspace mutations + action history/revert + AGPL/free-software transparency. Concretely: external-agent actions must be scoped; Liquid workspace mutations must be attributed and revertible; Meta-Harness config changes must be policy-gated and audited; Flow Agent runtime capabilities must be declared. Fixture execution evaluates them deterministically in process; productive Tool isolation requires the M1.2 Executor boundary. Because Watershed is AGPL/free software, users can **inspect, self-host, fork and verify** core behavior — transparency is part of the trust boundary, not a substitute for it.

## Principle: scripts define; enforcement must match the claim

Scripts are the single human-readable capability policy (allowed commands, parameters, exact read-only/writable mounts, runtime-read profile and network egress). The harness **compiles** each script into a runtime policy per Flow. Fixture evaluation is not an OS security boundary; productive Tool execution applies the compiled policy through a Flow-owned Executor and OS Sandbox backend. Allowlisting alone is *not* a boundary.

This paragraph governs Flow Agent scripts. Liquid Apps use the parallel principle defined below: App manifests declare capabilities, and the App Runtime plus Role and capability checks enforce them.

### M1.2 Tool execution trust boundary

M1.2 separates policy orchestration from isolation mechanics without separating security responsibility:

1. Before durable productive Run reservation, Flow Agent validates the selected administrator-owned Executor and backend readiness. For each Tool call it validates the request against the selected Building Blocks, compiles the canonical policy, synchronizes durable intent and validates the bounded response.
2. The Executor understands the versioned Flow request, rejects unsupported policy, translates it to one Sandbox backend, owns one Tool process tree and returns a canonical enforcement receipt bound to the exact applied-policy digest.
3. The Sandbox backend constructs the actual filesystem, process and deny-all network boundary. The Tool and all descendants are untrusted and must remain inside it.

Provider connections stay in Flow Agent outside the Tool Sandbox; Tool deny-all networking therefore does not prevent a remote provider or local model endpoint. The standard installation includes the official Default Sandbox Executor. On the supported Ubuntu 24.04 x64 platform, an administrator may explicitly opt out and configure a Custom Executor, which joins that installation's trusted computing base. Productive Tool execution on every other platform fails before Executor selection or Tool spawn; provider-only Flows do not use this boundary. Flow Agent can detect known protocol/setup mismatches but cannot prove that third-party evidence is truthful and makes no third-party compatibility or security guarantee.

Building Blocks, provider output and Workspace-local files cannot select or replace an Executor. A missing, incompatible, unsupported or failed Executor/backend prevents productive Tool spawn and has no fallback or escalation path. The Fixture executor remains available for deterministic tests and carries no OS-isolation claim. [`PROTOCOL.md`](PROTOCOL.md#m12-executor-protocol-adr-0146-adr-0160-adr-0161) owns the process contract; the [architecture concept](docs/concept/flow-agent-executor-architecture.md) explains the responsibility split.

Because scripts are human-reviewable security/capability artifacts, they pass through one private `core-script` Safe-YAML parser into one unambiguous model (ADR-0031, ADR-0061). It accepts one YAML 1.2 document and rejects duplicate or merge keys, anchors, aliases, explicit tags, nulls, unknown fields and configured resource-budget violations; there is no fallback parser. The checked-in JSON Schema files document the intended shape, existing semantic and registry validation remains authoritative, and the Flow Agent V-Spec defines canonical bytes.

Registry access starts from one opened capability for the Global Flow home. Loading opens every registry directory and YAML leaf without following links. M1.1 authoring must open or create each component relative to its already-open parent without following symbolic links or Windows reparse points, use exclusive no-replace creation, and verify the opened object's type and identity before descent; it never follows a successful path check with an ambient path reopen. Linux and macOS are the primary targets; the private boundary remains portable to Windows (ADR-0063, ADR-0064).

Flow Agent configuration, registry and runtime state live in the private user-global home defined by `PROTOCOL.md`. Workspace `.flow/config.yaml`, Workspace registries and other ambient project configuration have no technical authority and are never probed, merged or used as fallback. Missing, invalid, inaccessible, unsafe or conflicting global state fails before Run/session mutation. Optional global-home and harness-start Workspace `AGENTS.md` files remain a separate bounded instruction/context channel; their content cannot configure providers, models, registries, Runtime bindings, credentials or resource policy. The runtime store uses owner-only permissions or current-user-only Windows protection and fails closed when that boundary cannot be established. The `session.lock` leaf is only a persistent observable marker. Marker mutation cannot grant or revoke ownership, and process exit releases operating-system leases automatically. This is concurrency control, not OS isolation: a peer with the same OS identity can still corrupt runtime state or tamper with leases, and cross-host or durable ownership remains post-M1. Distinct canonical Workspace paths deliberately use distinct runtime stores, never distinct Flow configuration authorities.

Complete-history validation creates only a private per-command scratch index in that Workspace store; it is never conversation state. Unsafe scratch or insufficient admitted space fails before provider or Tool effects. Success, error and later crash recovery remove only the opened index identity and its verified files; replacement, links, unexpected names or mismatched identities fail closed without deleting foreign bytes. Cleanup assumes cooperative writers under the same OS identity; hostile same-identity replacement is outside the M1.1 guarantee.

Conversation storage maintenance and internal failed-creation rollback are defined in [`PROTOCOL.md`](PROTOCOL.md#local-run-storage-and-m11-conversation-trees).

- **Command allowlisting limits names, not effects.** Interpreters and deploy commands (`python`, `cf push`, build scripts, git hooks) are Turing-complete escapes; argument filters are bypassable via path traversal, symlinks and shell metacharacters.
- **Agent intent is untrusted (prompt injection / confused-deputy).** Reading untrusted content + holding private data + an exfiltration path is unconditionally exploitable regardless of prompt hardening ("lethal trifecta"). The fix is architectural separation, not a longer allowlist.

`flow-context-v0` always includes base runtime/security instructions in mandatory Tier 0 and fails before provider contact if they do not fit (ADR-0058). This protects instruction integrity and provider-cache consistency, but prompt text remains defense in depth. M1 policy evaluation is a deterministic correctness boundary, not process isolation; M1.2 OS enforcement is the security boundary for real tool processes.

## Provider authentication

The exact Flow-owned OAuth wire, cache, refresh and local-logout lifecycle is canonical in [`PROTOCOL.md`](PROTOCOL.md#m11-codex-subscription-provider). Flow Agent owns that current-user-protected cache, never imports another client's credentials, and never inserts its own provider credentials, account ids or authentication bodies into events, conversation history, Run Logs, diagnostics, exports or Tool environments. A definitive provider failure may persist and display the provider's direct message under the bounded contract in `PROTOCOL.md`; provider-supplied content remains the provider's responsibility. ADR-0107 selects the provider parser and temporal bounds; their evidence in the [M1.1 budget matrix](flow-agent/benchmarks/M1_1_BUDGETS.md) must pass before productive behavior is enabled.

Agentic Engineers configure Building Blocks and their capability limits; other users may run those predefined Flows. Flow Agent neither warns nor asks for confirmation before an assigned Tool runs. The Executor confines Tool processes and descendants; provider authentication remains Flow-owned and is never forwarded in an Execution request. Before provider or Tool dispatch, Flow Agent synchronizes durable intent; an intent without a terminal result is uncertain and never retries automatically. The exact Tool reconciliation command is defined in `PROTOCOL.md`.

## Accepted post-M1.1 target: local inference and portable continuation

This section defines unimplemented requirements; current M1.1 has neither Runtime bindings nor Portable continuation.

A future local model endpoint changes availability and data movement, not trust. Model output remains untrusted; the same typed values, context bounds, Tool validation, durable intent and sandbox policy must apply whether inference is local or remote. The local inference process is executable host code outside the Tool Sandbox; D-061 must decide whether it joins the trusted computing base or runs behind an enforceable identity/filesystem/network/secret boundary. Artifact signatures prove provenance, not confinement, and no productive offline-isolation claim exists until that boundary is implemented and tested. A Runtime binding may name only a typed credential reference bound to the selected provider and endpoint audience; Flow Agent must reject any mismatch before resolving credential material. Credential material stays in the Flow-owned store and retains its locking, refresh, protection and redaction contract. Provider output must not select the binding's endpoint, model/runtime artifacts, credential reference or resource policy, and a binding must not become Conversation authority or silently widen a Flow's capabilities. Offline-after-provisioning behavior and local model/runtime supply-chain evidence remain [D-059](docs/decisions/open-decisions.html#d-059).

Portable continuation must transfer verified context, not authority. Standalone Flow Agent must authenticate the destination's local OS actor, revalidate the selected Flow and capability/policy envelope, admit required resources and create a new child Run before effects. Integrations must independently reauthorize their own resources; accessing Liquid additionally requires the destination's effective Role and session grant. The archive must exclude credential-store records, Runtime-binding credential references, approvals and host-local leases and grants no implicit right to a Workspace or external system. Conversation content is still sensitive and may contain secrets previously supplied or exposed by users, providers or Tools; D-058 must define archive access, confidentiality and transfer protection and cannot promise secret-free bytes. Content hashes alone do not authenticate provenance: D-062 must bind the canonical root to an authenticated identity or classify the import as unauthenticated, non-executable evidence. Completed provider and Tool effects must not be redispatched; uncertain attempts retain the existing fail-closed reconciliation boundary. Direct private-store copying is not an import mechanism. Archive and branch rules remain [D-058](docs/decisions/open-decisions.html#d-058).

An offline device cannot learn a remote revocation while disconnected. No design may claim otherwise. Prior approvals must never transfer with a Conversation. Whether a destination may issue narrowly scoped local approvals while offline, and how expiry, one-time use and later revocation interact, remains [D-060](docs/decisions/open-decisions.html#d-060). Until that contract exists, imported sessions receive no portable approval authority.

## MVP VCS boundary

Flow Agent runs inside normal Git projects in the MVP, but it does not own project history and does not implement project VCS behavior. Auditability comes from deterministic Flow state, structured logs, protocol events, config-review/audit records and policy decisions; it does not come from an OS sandbox. Host Git may run only as an explicitly declared Tool and receives the same isolation as every other real command.

## Enforcement (per flow)

1. Compile script policy → deterministic fixture evaluation plus canonical OS policy artifacts. The official Executor maps exact pre-opened mount capabilities to stock Bubblewrap namespaces/mounts plus seccomp on Ubuntu 24.04 x64. Official productive macOS execution remains fail-closed pending the post-M1.2 review defined by ADR-0160. No supported path degrades to a weaker or unsandboxed backend.
2. **Network egress deny-by-default**. M1 performs no real network access, rejects non-empty Linux-target CIDR allow entries and emulates deny-all decisions for negative fixtures (ADR-0051, ADR-0052). M1.2 isolates Tool networking and supports deny-all only; positive grants remain disabled until D-046 selects and proves a finite enforcement design.
3. **Exact filesystem capabilities.** Each `read_only_mounts` entry is an exact read-only mount and each `writable_mounts` entry an exact writable mount. Sources are opened without following links and passed through fixed inherited-descriptor slots; replacement after validation cannot change the mounted object. No broad writable root plus exclusion grammar is part of the M1.2 boundary.
4. **Blast-radius control** via least-capability tools, deterministic logs and short-lived bounded runs. These controls do not imply process isolation.
5. **Runtime reads and lifecycle.** `runtime_profile: exact` is the default and exposes only the readiness-advertised bounded executable/interpreter/library objects. `host-system-read` is an explicit Agentic Engineer choice that adds only the Executor's fixed reviewed read-only Ubuntu system roots. Flow pre-opens every advertised source and binds its identity and Sandbox target into the policy digest; the Executor cannot add undeclared paths. Flow users, providers and Tools cannot alter or escalate the profile, and no automatic fallback exists. The statically linked official Executor needs no broad bootstrap runtime; a dynamic official artifact fails readiness. The Executor bounds I/O and time, confines descendants, terminates the complete process tree, validates enforcement on every invocation and persists the receipt with the terminal Tool attempt. Exact limits remain canonical in the M1.1 budget matrix.
6. Container, VM, micro-VM and remote backends are post-M1.2 integration candidates. They may implement the Executor contract after independent review, but their existence creates no Flow Agent support or security claim.

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
- Dependency hygiene: committed lockfiles, exact pins and minimal dependencies; CI runs `cargo audit` (RustSec advisories), `cargo deny` (license/bans/sources/advisory policy via `deny.toml`) and `pnpm audit --lockfile-only`. `cargo vet` remains an optional later addition. Rust reduces but does not eliminate supply-chain risk (`build.rs`/proc-macros run at build time); future isolated runtimes must limit blast radius regardless of language.

## M0/M1 policy-emulation scope

The M0 security packet describes policy artifacts and sandbox-negative tests for forbidden writes, network egress, out-of-phase tools, symlink traversal and interpreter misuse; it does not implement an OS sandbox. M1 checks the compiled policy through deterministic in-process execution/emulation of modeled decisions. The `agent-negative` predefined command maps fixture operation labels to expected deny reasons, and the out-of-phase fixture uses registry phase/tool shape rather than prompt prose. Own-script fixture writes remain in-process behavior and create new output leaves only; any existing target rejects before runtime or temporary-file mutation (ADR-0087). Existing-output replacement requires the finite post-M1 metadata contract in D-056. The artifact target is `linux-bubblewrap-seccomp`, matching M1.2's sole productive Ubuntu 24.04 x64 Bubblewrap-plus-seccomp backend; artifact compilation alone is not OS enforcement.

### M0 policy artifact contract

M0 policy artifacts are canonical JSON review/test outputs from `core-policy`, not OS-enforced sandboxes. One fixture per scenario is checked in under `core/core-policy/fixtures/<fixture-name>/` and compiled for the sole `linux-bubblewrap-seccomp` target.

Policy artifact serialization is UTF-8 JSON with lexicographically sorted object keys at every level, deterministic array order, no insignificant whitespace, LF line ending and final LF.

Policy artifact arrays are ordered after registry resolution and path normalization:

- `commands`: ascending by `tool_id`.
- `commands[].allowed_parameters`: ascending by `name`; each `allowed_values` array is ascending lexicographic order.
- `commands[].filesystem` arrays: ascending lexicographic order of normalized strings.
- `commands[].environment.allow`: ascending lexicographic order.
- `commands[].network.allow`: ascending by `transport`, then `cidr`, then numeric `port`.
- `phase_scope`: ascending by `phase_id`; each `tool_ids` array is ascending lexicographic order.
- Future policy-artifact arrays must either preserve schema-declared order or define an explicit sort key before they can appear in checked-in fixtures.

Top-level policy fields:

- `policy_version`: fixed string `"0"`.
- `target`: fixed string `"linux-bubblewrap-seccomp"`.
- `source_flow_definition_id`: resolved Flow definition id from the building-block registry.
- `commands`: array of `{ tool_id, tool_kind, command_id, executable, argv, script_runtime, allowed_parameters, environment, filesystem, network }`. `command_id` is the resolved predefined-command id (`^[a-z][a-z0-9_-]{0,63}$`) or `script:<tool-id>` for own-script tools. `executable` is the registry-resolved executable identity for `predefined-command` or the fixed runner for `own-script`. The Fixture executor emulates only the declared policy decision. Productive execution passes the verified executable and literal argument vector to the Executor without `PATH` lookup, shell parsing, environment expansion or glob expansion; POSIX shell parsing inside the reviewed own-script body remains part of own-script semantics. Registry semantic validation rejects U+0000 in every execution-vector string before process spawn. `script_runtime` is present only for `own-script` Tools and is `posix-sh`. Capabilities remain scoped to one `tool_id`; never infer that one Tool can use another Tool's mounts or runtime profile.
- `commands[].allowed_parameters`: array of parameter specs with `name` (exact flag), `value_type` (`none`, `string`, `integer`, `workspace-relative-path` or `enum`), `required` boolean and type-specific constraints. `allowed_values` is present only for `enum`; `value_pattern` and `max_length` are required for `string` and optional for `workspace-relative-path`; `min`/`max` are optional for `integer`. The M1 CLI/stub supplies no invocation parameter tokens; once a runtime interface supplies them, unknown parameters, extra positional arguments and invalid values are denied before launch.
- `commands[].environment`: `{ default, allow }`, where `default` is `clear` and `allow` contains exact host environment variable names explicitly selected by the Agentic Engineer. Tool processes never inherit the host environment by default, and policy artifacts/events never serialize environment values. V0 allow names match `^[A-Z_][A-Z0-9_]{0,63}$`. Flow Agent does not classify a syntactically valid name as a credential, secret or execution-control input and makes no automatic credential-detection or credential-protection claim for a configured value; responsibility follows the Building Block configuration. M1.1 compiles an empty allowlist and therefore passes no host environment values to productive Tools.
  M1 script compilation always emits `allow: []`.
- `commands[].filesystem`: exact `read_only_mounts` and `writable_mounts` string arrays after registry resolution plus `runtime_profile` (`exact` by default or explicit `host-system-read`). The M1.2 translation and descriptor rules are canonical in the [Executor protocol](PROTOCOL.md#m12-executor-protocol-adr-0146-adr-0160-adr-0161).
- `commands[].network`: `{ default, allow }`, where `default` is `deny` and `allow` is an array of typed network allow entries.
- `phase_scope`: array of `{ phase_id, tool_ids }` proving out-of-phase tools can be denied.
- `runtime_limits`: `{ headless, timeout_ms }`; `headless` is boolean and `timeout_ms` is an integer.

Network allow entries are CIDR objects, not strings:

- CIDR form: `{ kind: "cidr", transport, cidr, port }`;
- `transport` is `tcp` or `udp`;
- `port` is an integer from 1 to 65535;
- `cidr` is canonical CIDR notation; IP literals are represented as `/32` or `/128` CIDR entries;
- schemes, hostnames, DNS lookups, CNAME handling, suffix matching and wildcard matching are not part of the M0 network policy artifact.

V0 network policy grammar is IP/CIDR based. A future isolated tool may not rely on the policy compiler to resolve hostnames. The `linux-bubblewrap-seccomp` policy rejects non-empty allowlists (ADR-0051); fixture execution performs no DNS, DoH or DoT traffic. Any future CIDR/port grant requires enforceable egress controls and is reviewed as general network egress, not hostname-scoped access.

Negative expected-decision artifacts use the same canonical JSON serialization and contain `{ fixture_name, attempt, expected, reason_code, side_effects_allowed }`. `expected` is `deny`; `side_effects_allowed` is `false`; `reason_code` is one of `write_denied`, `network_denied`, `environment_denied`, `tool_out_of_phase`, `symlink_escape_denied` or `interpreter_escape_denied`.

`attempt` is a discriminated object. No additional attempt fields are allowed in v0:

- forbidden write/create: `{ kind: "write", tool_id, operation, path }`, where `operation` is `write` or `create`;
- forbidden rename: `{ kind: "write", tool_id, operation: "rename", from_path, to_path }`;
- forbidden network: `{ kind: "network", tool_id, transport, destination, port }`, where `destination` is a hostname, IP literal or CIDR string attempted by the fixture and `transport`/`port` use the network allow-entry rules;
- forbidden environment read: `{ kind: "environment", tool_id, name }`;
- out-of-phase tool: `{ kind: "tool_out_of_phase", phase_id, tool_id }`;
- symlink escape: `{ kind: "symlink_escape", tool_id, operation, path, symlink_path, symlink_target }`;
- interpreter escape: `{ kind: "interpreter_escape", tool_id, executable, argv }`, where `argv` is an array of strings.
