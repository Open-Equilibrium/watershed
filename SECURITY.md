# Security

Cross-cutting security model for all tools. Do not re-decide these per tool.

## Reporting a vulnerability

Report suspected vulnerabilities privately to **b-weber@gmx.at** — please do not open public issues for security problems. Include reproduction steps and affected files/components where possible. Reports are handled on a **best-effort basis**: this project gives **no guarantees** of response time, fixes, or any warranty of any kind; the software is provided "as is" (see `LICENSE`, AGPL-3.0-only §15–16). Coordinated disclosure is appreciated.

## Trust model

Watershed's defensible trust model is the combination across its three layers: structured flows + scoped runtime capabilities + normalized events/transcripts + policy gates + metric feedback + permissioned workspace mutations + action history/revert + AGPL/free-software transparency. Concretely: external-agent actions must be scoped; Liquid workspace mutations must be attributed and revertible; Meta-Harness config changes must be policy-gated and audited; Flow Agent runtime capabilities must be declared. M1 evaluates them deterministically in process for fixture-bounded execution; only M1.2 OS backends establish an isolation boundary. Because Watershed is AGPL/free software, users can **inspect, self-host, fork and verify** core behavior — transparency is part of the trust boundary, not a substitute for it.

## Principle: scripts define; enforcement must match the claim

Scripts are the single human-readable capability policy (allowed commands, parameters, read/write roots, network egress). The harness **compiles** each script into a runtime policy per Flow. M1 checks and emulates the artifact in process for fixture-bounded execution; this is not an OS security boundary. M1.1 adds bounded external execution without claiming isolation. M1.2 must apply the same compiled policy through OS backends. Allowlisting alone is *not* a boundary.

This paragraph governs Flow Agent scripts. Liquid Apps use the parallel principle defined below: App manifests declare capabilities, and the App Runtime plus Role and capability checks enforce them.

Because scripts are human-reviewable security/capability artifacts, they pass through one private `core-script` Safe-YAML parser into one unambiguous model (ADR-0031, ADR-0061). It accepts one YAML 1.2 document and rejects duplicate or merge keys, anchors, aliases, explicit tags, nulls, unknown fields and configured resource-budget violations; there is no fallback parser. The checked-in JSON Schema files document the intended shape, existing semantic and registry validation remains authoritative, and the Flow Agent V-Spec defines canonical bytes.

Registry loading starts from one opened workspace capability and opens every registry directory and YAML leaf without following links. Linux and macOS are the primary targets; the private boundary remains portable to Windows (ADR-0063, ADR-0064).

M1 session ownership treats direct local mutation of `.flow` as in scope. The sole authority is an exclusive OS-held lease in the workspace-adjacent coordinator defined by `PROTOCOL.md`; the workspace `.lock` leaf is only a persistent observable marker. Marker mutation cannot grant or revoke ownership, and process exit releases the lease automatically. This is concurrency control, not OS isolation: the canonical workspace parent is trusted, a peer with the same OS identity can still corrupt workspace data or tamper with the coordinator, and cross-host/durable ownership remains post-M1.

- **Command allowlisting limits names, not effects.** Interpreters and deploy commands (`python`, `cf push`, build scripts, git hooks) are Turing-complete escapes; argument filters are bypassable via path traversal, symlinks and shell metacharacters.
- **Agent intent is untrusted (prompt injection / confused-deputy).** Reading untrusted content + holding private data + an exfiltration path is unconditionally exploitable regardless of prompt hardening ("lethal trifecta"). The fix is architectural separation, not a longer allowlist.

`flow-context-v0` always includes base runtime/security instructions in mandatory Tier 0 and fails before provider contact if they do not fit (ADR-0058). This protects instruction integrity and provider-cache consistency, but prompt text remains defense in depth. M1 policy evaluation is a deterministic correctness boundary, not process isolation; M1.2 OS enforcement is the security boundary for real tool processes.

## MVP VCS boundary

Flow Agent runs inside normal Git projects in the MVP, but it does not own project history and does not implement project VCS behavior. M1 auditability comes from deterministic Flow state, structured logs, protocol events, config-review/audit records and policy decisions; it does not come from an OS sandbox. M1.1 may run host Git only when explicitly declared as a Tool command through the bounded runner. M1.2 must isolate it like every other real command.

## Enforcement (per flow)

1. Compile script policy → deterministic M1 in-process evaluation plus OS policy artifacts. M1.2 adds Linux Landlock + seccomp and macOS Seatbelt backends. Reuse proven isolation primitives where possible. M0 produces policy artifacts plus escape tests (ADR-0032, ADR-0052).
2. **Network egress deny-by-default**. M1 performs no real network access, rejects non-empty Linux-target CIDR allow entries and emulates deny-all decisions for negative fixtures (ADR-0051, ADR-0052). M1.2 must enforce egress before positive grants can be claimed.
3. M1 checks and emulates fixture **read/write policy** against declared roots and protected paths. M1.2 must confine real process access at the OS boundary.
4. **Blast-radius control** via least-capability tools, deterministic logs and short-lived bounded runs. These controls do not imply process isolation.
5. M1.1 subprocesses must be bounded, headless and timed out — for stability, **not** as a security boundary.
6. Optional **container/microVM per Flow** is M1.2 work and does not replace the required OS baseline unless a later decision changes it.

## Meta-Agent configuration writes

A Meta-Agent may reconfigure underlying agents, **policy-gated**: low-risk changes apply only through the configured review/audit flow; sensitive changes (permissions, tools, network, schedules, external credentials) require human approval. Every change is recorded with who/what/when, is monitorable and is revertible according to the chosen config-storage model. The human always knows what changed.

The same gate applies to **all Meta-Harness control surfaces** — its CLI, API/service and BYOA/external command surface. Meta-Harness runs headlessly (without Liquid), so the policy/audit gate, not a UI confirmation dialog, is the boundary: sensitive commands from any client must be authorized and audited identically.

Execution ownership is host-local. A Meta-Harness executor may control only CLI processes created or adopted on its own host under an explicit local identity; it rejects cross-host process claims. Exposing the API to another device requires authenticated, integrity-protected transport and does not expand executor authority. Liquid must route live commands to the instance that owns the addressed session/configuration and must not treat cached state as controllable while that instance is unreachable.

## Liquid workspace access & external-agent edits

Liquid is a standalone workspace product that external agents and tools can read and edit through its workspace CLI/API. That access is permissioned and auditable:

- CLI/API access requires an authenticated identity and an assigned allow-only **Role**. Unlisted resources and actions are denied by default; explicit deny/blacklist rules are deferred.
- Roles may be assigned to users, groups, agent profiles, sessions and Automations and may allow discovery, proposal, execution, approval or management over named Workspaces, Pages, Blocks, Sources, App actions and Meta-Harness projections.
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
- Dependency hygiene: lockfiles + pinning, vendoring, minimal dependencies, and a CI gate of `cargo audit` (RustSec advisories) + `cargo deny` (license/bans/sources/advisory policy via `deny.toml`). Both run as **mandatory M0 CI gates** (ADR-0021); `cargo vet` remains an optional later addition. Rust reduces but does not eliminate supply-chain risk (`build.rs`/proc-macros run at build time); future isolated runtimes must limit blast radius regardless of language.

## M0/M1 policy-emulation scope

The M0 security packet describes policy artifacts and sandbox-negative tests for forbidden writes, network egress, out-of-phase tools, protected paths, symlink traversal and interpreter misuse; it does not implement an OS sandbox. M1 checks the compiled policy through deterministic in-process execution/emulation of modeled decisions. The `agent-negative` predefined command maps fixture operation labels to expected deny reasons, and the out-of-phase fixture uses registry phase/tool shape rather than prompt prose. Own-script fixture writes remain in-process behavior. M1.2 targets Linux Landlock/seccomp enforcement and macOS Seatbelt parity (ADR-0052).

### M0 policy artifact contract

M0 policy artifacts are canonical JSON review/test outputs from `core-policy`, not OS-enforced sandboxes. One fixture per scenario is checked in under `core/core-policy/fixtures/<fixture-name>/`; tests instantiate it for both `linux-landlock-seccomp` and `macos-seatbelt`. Add target-specific fixtures only when their outputs differ.

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
- `target`: `linux-landlock-seccomp` or `macos-seatbelt`.
- `source_flow_definition_id`: resolved Flow definition id from the building-block registry.
- `commands`: array of `{ tool_id, tool_kind, command_id, executable, argv, script_runtime, allowed_parameters, environment, filesystem, network }`. `command_id` is the resolved predefined-command id (`^[a-z][a-z0-9_-]{0,63}$`) or `script:<tool_id>` for own-script tools. `executable` is the registry-resolved executable identity for `predefined-command` or the fixed runner for `own-script`. M1 does not launch either form: its fixture executor only emulates the declared policy decision. M1.1 predefined-command launch direct-execs the registry-resolved executable with literal `argv` and never uses PATH lookup, shell parsing, environment expansion or glob expansion. M1.1 own-script launch direct-execs the fixed `posix-sh` runner; POSIX shell parsing/expansion inside the reviewed `script_body` is part of own-script semantics, but runner path and runner arguments are not script-controllable. `argv` is the literal base argument vector before validated allowed-parameter tokens. `script_runtime` is present only for `own-script` tools and is `posix-sh`; it is omitted for `predefined-command` tools. M1.2 applies the OS isolation boundary to both launch forms. Capabilities are scoped to this `tool_id`; never infer that one tool can use another tool's grants.
- `commands[].allowed_parameters`: array of parameter specs with `name` (exact flag), `value_type` (`none`, `string`, `integer`, `workspace-relative-path` or `enum`), `required` boolean and type-specific constraints. `allowed_values` is present only for `enum`; `value_pattern` and `max_length` are required for `string` and optional for `workspace-relative-path`; `min`/`max` are optional for `integer`. The M1 CLI/stub supplies no invocation parameter tokens; once a runtime interface supplies them, unknown parameters, extra positional arguments and invalid values are denied before launch.
- `commands[].environment`: `{ default, allow }`, where `default` is `clear` and `allow` is an array of non-secret environment variable names. Tool processes never inherit the host environment by default, and policy artifacts/events never serialize environment values. V0 allow names match `^[A-Z_][A-Z0-9_]{0,63}$` and must not match secret-bearing prefixes or words: `*_TOKEN`, `*_KEY`, `*_SECRET`, `*_PASSWORD`, `*_CREDENTIAL*`, `AWS_*`, `GCP_*`, `AZURE_*`, `OPENAI_*`, `ANTHROPIC_*`, `GH_*`, `GITHUB_*`, `CF_*` or `KUBE*`. Environment allowlists also deny execution-control, proxy, VCS/helper-control, config-injection and credential-handle names: `PATH`, `PATHEXT`, `LD_*`, `DYLD_*`, `BASH_ENV`, `ENV`, `SHELLOPTS`, `IFS`, `CDPATH`, `GLOBIGNORE`, `NODE_OPTIONS`, `PYTHONPATH`, `PYTHONHOME`, `RUBYOPT`, `PERL5LIB`, `PERL5OPT`, `JAVA_TOOL_OPTIONS`, `RUSTC_WRAPPER`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_*_RUNNER`, `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, `FTP_PROXY`, `GIT_SSH_COMMAND`, `GIT_ASKPASS`, `SSH_ASKPASS`, `GIT_CONFIG_*`, `GIT_EXEC_PATH`, `GIT_TEMPLATE_DIR`, `GIT_TERMINAL_PROMPT`, `GIT_PROXY_COMMAND`, `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `GPG_TTY`, `KRB5CCNAME`, `DOCKER_HOST`, `DOCKER_CONFIG`, `KUBECONFIG`, `NETRC` and `NPM_CONFIG_USERCONFIG`. Any future need for those classes must be modeled as a separate reviewed capability, not a generic environment allow entry.
  M1 script compilation always emits `allow: []`.
- `commands[].filesystem`: `{ read_roots, write_roots, protected_paths, protected_path_grants }`, each a string array after registry resolution.
- `commands[].network`: `{ default, allow }`, where `default` is `deny` and `allow` is an array of typed network allow entries.
- `phase_scope`: array of `{ phase_id, tool_ids }` proving out-of-phase tools can be denied.
- `runtime_limits`: `{ headless, timeout_ms }`; `headless` is boolean and `timeout_ms` is an integer.

Network allow entries are CIDR objects, not strings:

- CIDR form: `{ kind: "cidr", transport, cidr, port }`;
- `transport` is `tcp` or `udp`;
- `port` is an integer from 1 to 65535;
- `cidr` is canonical CIDR notation; IP literals are represented as `/32` or `/128` CIDR entries;
- schemes, hostnames, DNS lookups, CNAME handling, suffix matching and wildcard matching are not part of the M0 network policy artifact.

V0 network policy grammar is IP/CIDR based. A future isolated tool may not rely on the policy compiler to resolve hostnames. For M1 Linux-target policy, non-empty allowlists are rejected (ADR-0051); fixture execution performs no DNS, DoH or DoT traffic. Any future CIDR/port grant requires enforceable M1.2 egress controls and is reviewed as general network egress, not hostname-scoped access.

Negative expected-decision artifacts use the same canonical JSON serialization and contain `{ fixture_name, attempt, expected, reason_code, side_effects_allowed }`. `expected` is `deny`; `side_effects_allowed` is `false`; `reason_code` is one of `write_denied`, `network_denied`, `environment_denied`, `tool_out_of_phase`, `protected_path_denied`, `symlink_escape_denied` or `interpreter_escape_denied`.

`attempt` is a discriminated object. No additional attempt fields are allowed in v0:

- forbidden write/create: `{ kind: "write", tool_id, operation, path }`, where `operation` is `write` or `create`;
- forbidden rename: `{ kind: "write", tool_id, operation: "rename", from_path, to_path }`;
- forbidden network: `{ kind: "network", tool_id, transport, destination, port }`, where `destination` is a hostname, IP literal or CIDR string attempted by the fixture and `transport`/`port` use the network allow-entry rules;
- forbidden environment read: `{ kind: "environment", tool_id, name }`;
- out-of-phase tool: `{ kind: "tool_out_of_phase", phase_id, tool_id }`;
- protected-path access: `{ kind: "protected_path", tool_id, operation, path }`, where `operation` is `read`, `write`, `create` or `execute`;
- protected-path rename: `{ kind: "protected_path", tool_id, operation: "rename", from_path, to_path }`;
- symlink escape: `{ kind: "symlink_escape", tool_id, operation, path, symlink_path, symlink_target }`;
- interpreter escape: `{ kind: "interpreter_escape", tool_id, executable, argv }`, where `argv` is an array of strings.

Default protected paths are denied even inside a declared read/write root unless a flow explicitly grants the exact path or pattern:

Protected-path matching semantics:

- Convert `\` to `/`, remove duplicate separators and reject absolute paths, drive prefixes and paths that escape the declared root.
- Resolve paths component-by-component from the declared root before matching, with symlinks resolved before applying any following `..` component. This is the `openat2`/realpath-style order; lexical `..` cleanup alone is not authoritative.
- For create, rename or write requests to a non-existent leaf, resolve every existing ancestor including the final parent component-wise, then append the unresolved suffix for matching. Symlink flows, unresolved symlink targets and root escapes are denied.
- Compare both the normalized lexical request and the component-wise resolved absolute and workspace-root-relative forms. A request is denied if any form matches a protected pattern and no explicit grant matches.
- Glob grammar is limited to `*` (any characters except `/`), `?` (one character except `/`) and `**` (zero or more complete path segments). No brace expansion, extglobs or regex syntax.
- Patterns match the whole normalized path. A pattern starting with `**/` may match at any depth; otherwise matching is anchored at the beginning.
- Matching is case-sensitive on Linux targets and conservatively ASCII case-insensitive on macOS Seatbelt targets, regardless of the host volume's case setting.
- Explicit grants are tool-scoped entries in `commands[].filesystem.protected_path_grants`. A grant only removes the protected-path deny for that tool; the path must still be inside the same tool's declared read/write scope.

- repo/runtime metadata: `**/.git`, `**/.git/**`, `**/.flow`, `**/.flow/**`;
- env/credential files: `**/.env`, `**/.env.*`, `**/*.env`, `**/*.local`, `**/.npmrc`, `**/.pypirc`, `**/.netrc`, `**/.git-credentials`;
- key material: `**/*.pem`, `**/*.key`, `**/*.p12`, `**/*.pfx`, `**/id_rsa`, `**/id_dsa`, `**/id_ecdsa`, `**/id_ed25519`, `**/id_ecdsa_sk`, `**/id_ed25519_sk`;
- credential stores/directories: `**/.ssh`, `**/.ssh/**`, `**/.gnupg`, `**/.gnupg/**`, `**/.aws`, `**/.aws/**`, `**/.azure`, `**/.azure/**`, `**/.docker`, `**/.docker/**`, `**/.kube`, `**/.kube/**`, `**/.config/gcloud`, `**/.config/gcloud/**`, `**/.config/gh`, `**/.config/gh/**`, `**/credentials`, `**/credentials/**`, `**/credentials.toml`, `**/secrets`, `**/secrets/**`.
