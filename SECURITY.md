# Security

Cross-cutting security model for all tools. Do not re-decide these per tool.

## Reporting a vulnerability

Report suspected vulnerabilities privately to **b-weber@gmx.at** — please do not open public issues for security problems. Include reproduction steps and affected files/components where possible. Reports are handled on a **best-effort basis**: this project gives **no guarantees** of response time, fixes, or any warranty of any kind; the software is provided "as is" (see `LICENSE`, AGPL-3.0-only §15–16). Coordinated disclosure is appreciated.

## Trust model

Watershed's defensible trust model is the combination across its three layers: structured loops + scoped runtime capabilities + normalized events/transcripts + policy gates + metric feedback + permissioned workspace mutations + action history/revert + AGPL/free-software transparency. Concretely: external-agent actions must be scoped; Liquid workspace mutations must be attributed and revertible; Meta-Harness config changes must be policy-gated and audited; Loop Agent runtime capabilities must be declared and sandboxed. Because Watershed is AGPL/free software, users can **inspect, self-host, fork and verify** core behavior — transparency is part of the trust boundary, not a substitute for it.

## Principle: scripts define, sandbox enforces

Scripts are the single human-readable capability policy (allowed commands, parameters, read/write roots, network egress). The harness **compiles** each script into a runtime policy per loop; M1 runs deterministic in-process execution/emulation for the modeled checks, and post-M1 OS backends must apply the same compiled policy. Allowlisting alone is *not* a boundary.

Because scripts are human-reviewable security/capability artifacts, they parse once to one unambiguous model through the private `core-script` Safe-YAML boundary (ADR-0031, ADR-0061). It accepts one YAML 1.2 document and rejects duplicate or merge keys, anchors, aliases, explicit tags, nulls, unknown fields and configured resource-budget violations; there is no fallback parser. The checked-in JSON Schema files document the intended shape, existing semantic and registry validation remains authoritative, and the Loop Agent V-Spec defines canonical bytes.

- **Command allowlisting limits names, not effects.** Interpreters and deploy commands (`python`, `cf push`, build scripts, git hooks) are Turing-complete escapes; argument filters are bypassable via path traversal, symlinks and shell metacharacters.
- **Agent intent is untrusted (prompt injection / confused-deputy).** Reading untrusted content + holding private data + an exfiltration path is unconditionally exploitable regardless of prompt hardening ("lethal trifecta"). The fix is architectural separation, not a longer allowlist.

`loop-context-v0` always includes base runtime/security instructions in mandatory Tier 0 and fails before provider contact if they do not fit (ADR-0058). This protects instruction integrity and provider-cache consistency, but prompt text remains defense in depth: compiled capability policy and runtime enforcement are the security boundary.

## MVP VCS boundary

Loop Agent runs inside normal Git projects in the MVP, but it does not own project history and does not implement project VCS behavior. Security and auditability in the MVP come from deterministic loop state, structured logs, protocol events, config-review/audit records and sandbox enforcement. Host Git operations may run only when explicitly declared as Tool commands and sandboxed like any other command.

## Enforcement (per loop)

1. Compile script policy → deterministic M1 in-process enforcement/emulation plus OS policy artifacts. Linux Landlock + seccomp and macOS Seatbelt are post-M1 OS-enforcement backends. Reuse proven sandbox primitives where possible. M0 produces policy artifacts plus escape tests (ADR-0032, ADR-0052).
2. **Network egress deny-by-default**. M1 Linux-target policy rejects non-empty CIDR allow entries and emulates deny-all network decisions for sandbox-negative tests (ADR-0051, ADR-0052). CIDR allow entries remain part of the policy artifact/schema so reviewed capabilities are explicit, but they are not silently treated as enforced by Landlock/seccomp until a post-M1 egress backend exists.
3. Filesystem **read/write confined** to declared roots; protect the default protected paths below unless explicitly granted.
4. **Blast-radius control** via least-capability tools, isolated workspaces when configured, deterministic logs and short-lived bounded runs.
5. Bounded/headless/timeout execution + `.loop/logs` — for stability, **not** a security boundary by itself.
6. Post-M1 optional **container/microVM per loop** for loops touching untrusted content (web, foreign repos).

## Meta-Agent configuration writes

A Meta-Agent may reconfigure underlying agents, **policy-gated**: low-risk changes apply only through the configured review/audit flow; sensitive changes (permissions, tools, network, schedules, external credentials) require human approval. Every change is recorded with who/what/when, is monitorable and is revertible according to the chosen config-storage model. The human always knows what changed.

The same gate applies to **all Meta-Harness control surfaces** — its CLI, API/service and BYOA/external command surface. Meta-Harness runs headlessly (without Liquid), so the policy/audit gate, not a UI confirmation dialog, is the boundary: sensitive commands from any client must be authorized and audited identically.

## Liquid workspace access & external-agent edits

Liquid is a standalone workspace product that external agents and tools can read and edit through its workspace CLI/API. That access is permissioned and auditable:

- CLI/API access requires explicit workspace permission; agent reads and writes are **scoped** (external-agent permission model: D-030).
- Every workspace write — from the UI, Liquid AI, the CLI/API or an external agent — goes through one **permissioned mutation pipeline** and is recorded in Liquid's **action history**; there are no hidden writes that bypass it (D-032).
- External-agent writes are **attributed** (actor/origin) and **revertible**; sensitive changes require approval, and a proposed diff can be reviewed before apply.
- The action history must be tamper-evident enough for product needs; exact cryptographic guarantees are open.
- Secrets/credentials stored in workspace data require special handling.
- Script components and external-agent edits are different risk classes and are treated separately (script component runtime/sandbox: D-034).

This is Liquid's **workspace** action history (over Liquid's own data), not a project-code VCS. Detail: [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html).

## Plugins & supply chain

- Post-M1 plugins run as **Wasmtime** modules: capability-scoped, sandboxed, with explicit grants and resource limits.
- Dependency hygiene: lockfiles + pinning, vendoring, minimal dependencies, and a CI gate of `cargo audit` (RustSec advisories) + `cargo deny` (license/bans/sources/advisory policy via `deny.toml`). Both run as **mandatory M0 CI gates** (ADR-0021); `cargo vet` remains an optional later addition. Rust reduces but does not eliminate supply-chain risk (`build.rs`/proc-macros run at build time); the runtime sandbox limits blast radius regardless of language.

## M0/M1 sandbox scope

The M0 security packet describes policy artifacts and sandbox-negative tests for forbidden writes, network egress, out-of-phase tools, protected paths, symlink traversal and interpreter misuse; it does not implement the OS sandbox yet. M1 applies the compiled policy through deterministic in-process execution/emulation of modeled decisions. The `agent-negative` predefined command maps fixture operation labels to expected deny reasons, and the out-of-phase fixture uses registry phase/tool shape rather than prompt prose. Own-script writes use the bounded fixture executor. Linux Landlock/seccomp OS enforcement and macOS Seatbelt parity are post-M1 targets (ADR-0052).

### M0 policy artifact contract

M0 policy artifacts are canonical JSON review/test outputs from `core-policy`, not OS-enforced sandboxes. Fixtures are checked in under `core/core-policy/fixtures/<fixture-name>/<target>.policy.json`, plus `<target>.expected.json` for sandbox-negative expected decisions. Targets are `linux-landlock-seccomp` and `macos-seatbelt`.

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
- `fixture_name`: fixture registry name, e.g. `hello-loop`.
- `target`: `linux-landlock-seccomp` or `macos-seatbelt`.
- `source_loop_definition_id`: resolved Loop definition id from the building-block registry.
- `commands`: array of `{ tool_id, tool_kind, command_id, executable, argv, script_runtime, allowed_parameters, environment, filesystem, network }`. `command_id` is the resolved predefined-command id (`^[a-z][a-z0-9_-]{0,63}$`) or `script:<tool_id>` for own-script tools. `executable` is the registry-resolved executable identity for `predefined-command` or the fixed runner for `own-script`. Predefined-command launch direct-execs the registry-resolved executable with literal `argv` and never uses PATH lookup, shell parsing, environment expansion or glob expansion. Own-script launch direct-execs the fixed `posix-sh` runner; POSIX shell parsing/expansion inside the reviewed `script_body` is part of own-script semantics and remains inside the sandbox, but runner path and runner arguments are not script-controllable. `argv` is the literal base argument vector before validated allowed-parameter tokens. `script_runtime` is present only for `own-script` tools and is `posix-sh`; it is omitted for `predefined-command` tools. Capabilities are scoped to this `tool_id`; never infer that one tool can use another tool's grants.
- `commands[].allowed_parameters`: array of parameter specs with `name` (exact flag), `value_type` (`none`, `string`, `integer`, `workspace-relative-path` or `enum`), `required` boolean and type-specific constraints. `allowed_values` is present only for `enum`; `value_pattern` and `max_length` are required for `string`; `min`/`max` are optional for `integer`. Unknown parameters, extra positional arguments and values that fail type/path validation are denied before command launch.
- `commands[].environment`: `{ default, allow }`, where `default` is `clear` and `allow` is an array of non-secret environment variable names. Tool processes never inherit the host environment by default, and policy artifacts/events never serialize environment values. V0 allow names match `^[A-Z_][A-Z0-9_]{0,63}$` and must not match secret-bearing prefixes or words: `*_TOKEN`, `*_KEY`, `*_SECRET`, `*_PASSWORD`, `*_CREDENTIAL*`, `AWS_*`, `GCP_*`, `AZURE_*`, `OPENAI_*`, `ANTHROPIC_*`, `GH_*`, `GITHUB_*`, `CF_*` or `KUBE*`. Environment allowlists also deny execution-control, proxy, VCS/helper-control, config-injection and credential-handle names: `PATH`, `PATHEXT`, `LD_*`, `DYLD_*`, `BASH_ENV`, `ENV`, `SHELLOPTS`, `IFS`, `CDPATH`, `GLOBIGNORE`, `NODE_OPTIONS`, `PYTHONPATH`, `PYTHONHOME`, `RUBYOPT`, `PERL5LIB`, `PERL5OPT`, `JAVA_TOOL_OPTIONS`, `RUSTC_WRAPPER`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_*_RUNNER`, `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, `FTP_PROXY`, `GIT_SSH_COMMAND`, `GIT_ASKPASS`, `SSH_ASKPASS`, `GIT_CONFIG_*`, `GIT_EXEC_PATH`, `GIT_TEMPLATE_DIR`, `GIT_TERMINAL_PROMPT`, `GIT_PROXY_COMMAND`, `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `GPG_TTY`, `KRB5CCNAME`, `DOCKER_HOST`, `DOCKER_CONFIG`, `KUBECONFIG`, `NETRC` and `NPM_CONFIG_USERCONFIG`. Any future need for those classes must be modeled as a separate reviewed capability, not a generic environment allow entry.
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

V0 network policy grammar is IP/CIDR based. A sandboxed tool may not rely on the policy compiler to resolve hostnames. For M1 Linux-target policy, non-empty allowlists are rejected (ADR-0051); direct DNS, DoH and DoT traffic is therefore denied in deterministic in-process runs. If a future CIDR/port grant is enforceable, it is reviewed as general network egress, not hostname-scoped access.

Negative expected-decision artifacts use the same canonical JSON serialization and contain `{ fixture_name, target, attempt, expected, reason_code, side_effects_allowed }`. `expected` is `deny`; `side_effects_allowed` is `false`; `reason_code` is one of `write_denied`, `network_denied`, `environment_denied`, `tool_out_of_phase`, `protected_path_denied`, `symlink_escape_denied` or `interpreter_escape_denied`.

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

Default protected paths are denied even inside a declared read/write root unless a loop explicitly grants the exact path or pattern:

Protected-path matching semantics:

- Convert `\` to `/`, remove duplicate separators and reject absolute paths, drive prefixes and paths that escape the declared root.
- Resolve paths component-by-component from the declared root before matching, with symlinks resolved before applying any following `..` component. This is the `openat2`/realpath-style order; lexical `..` cleanup alone is not authoritative.
- For create, rename or write requests to a non-existent leaf, resolve every existing ancestor including the final parent component-wise, then append the unresolved suffix for matching. Symlink loops, unresolved symlink targets and root escapes are denied.
- Compare both the normalized lexical request and the component-wise resolved absolute and workspace-root-relative forms. A request is denied if any form matches a protected pattern and no explicit grant matches.
- Glob grammar is limited to `*` (any characters except `/`), `?` (one character except `/`) and `**` (zero or more complete path segments). No brace expansion, extglobs or regex syntax.
- Patterns match the whole normalized path. A pattern starting with `**/` may match at any depth; otherwise matching is anchored at the beginning.
- Matching is case-sensitive on Linux targets and case-insensitive on macOS Seatbelt targets to match the default filesystem risk profile.
- Explicit grants are tool-scoped entries in `commands[].filesystem.protected_path_grants`. A grant only removes the protected-path deny for that tool; the path must still be inside the same tool's declared read/write scope.

- repo/runtime metadata: `**/.git`, `**/.git/**`, `**/.loop`, `**/.loop/**`, legacy `**/.flow`, legacy `**/.flow/**`;
- env/credential files: `**/.env`, `**/.env.*`, `**/*.env`, `**/*.local`, `**/.npmrc`, `**/.pypirc`, `**/.netrc`, `**/.git-credentials`;
- key material: `**/*.pem`, `**/*.key`, `**/*.p12`, `**/*.pfx`, `**/id_rsa`, `**/id_dsa`, `**/id_ecdsa`, `**/id_ed25519`, `**/id_ecdsa_sk`, `**/id_ed25519_sk`;
- credential stores/directories: `**/.ssh`, `**/.ssh/**`, `**/.gnupg`, `**/.gnupg/**`, `**/.aws`, `**/.aws/**`, `**/.azure`, `**/.azure/**`, `**/.docker`, `**/.docker/**`, `**/.kube`, `**/.kube/**`, `**/.config/gcloud`, `**/.config/gcloud/**`, `**/.config/gh`, `**/.config/gh/**`, `**/credentials`, `**/credentials/**`, `**/credentials.toml`, `**/secrets`, `**/secrets/**`.
