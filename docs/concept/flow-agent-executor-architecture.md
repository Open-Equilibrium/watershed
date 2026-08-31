# Flow Agent Executor and Sandbox architecture

This document explains the accepted M1.2 design. Normative milestone scope lives in [`PLAN.md`](../../PLAN.md), wire behavior in [`PROTOCOL.md`](../../PROTOCOL.md), security invariants in [`SECURITY.md`](../../SECURITY.md), evidence in [`TESTING.md`](../../TESTING.md), and terms in [`GLOSSARY.md`](../../GLOSSARY.md).

## Decision in one minute

Flow Agent validates a Tool request and compiles its policy. One short-lived Executor translates that policy and manages one Tool process tree. The Sandbox backend constructs the operating-system boundary. Provider traffic remains in Flow Agent, outside the Tool Sandbox.

The standard installation supplies an official, statically linked `flow-executor` sibling. An administrator may explicitly omit it or select an absolute Custom Executor override. Building Blocks, Flow users, providers, Tools, Workspaces and environment variables cannot select or replace the Executor. Failure never chooses a weaker path.

The official productive M1.2 backend is stock Bubblewrap plus seccomp on Ubuntu 24.04 x64. All productive Tool execution, including an administrator-owned Custom Executor, is limited to that platform. macOS, Windows and other targets fail before Executor selection or Tool spawn; provider-only Flows do not use this boundary. A Custom Executor receives no Flow Agent compatibility or security guarantee.

## Responsibility split

```mermaid
flowchart TD
  AE[Agentic Engineer] -->|authors Tool capabilities| FA[Flow Agent]
  U[Flow user] -->|runs predefined Flow| FA
  FA <--> P[Provider]
  FA -->|one bounded request| E[Executor]
  A[Administrator] -->|installs or selects| E
  E --> B[Sandbox backend]
  B --> T[Tool process and descendants]
  FA -. deterministic tests .-> F[Fixture executor]
  F --> PE[Policy emulation only]
```

| Owner | Responsibility |
|---|---|
| Agentic Engineer | Select the Tool identity, parameters, exact mounts, runtime-read profile and deny-all network policy. |
| Flow Agent | Validate authority, derive the selected Executor, prove readiness, compile canonical policy, persist attempt state, validate the response and fail closed. |
| Executor | Validate one request, translate its exact capabilities, manage one Tool process tree and return a bounded Tool result or typed error. |
| Sandbox backend | Construct and enforce filesystem, process and deny-all network isolation. |
| Administrator | Own the installed sibling or protected Custom Executor override and assess any third-party implementation. |

The Fixture executor remains in process, deterministic and independent of Sandbox installation. It is not a productive escape hatch and makes no isolation claim. Meta-Harness may supervise the whole `flow` CLI process but never selects or manages a Flow Tool Sandbox; Liquid has no responsibility in this boundary.

## Selection and readiness

```mermaid
flowchart TD
  S[Resolve Flow Agent executable] --> O{Protected Custom override?}
  O -->|yes| C[Open absolute Custom Executor]
  O -->|no| D[Open sibling flow-executor]
  C --> P[Probe protocol, platform, backend and runtime manifests]
  D --> P
  P -->|ready| R[Durably reserve productive Run]
  P -->|failure| F[Fail without Run or Tool spawn]
```

The standard installer includes the sibling unless the administrator chooses `--no-default-executor`. `flow executor configure --path <absolute-path>` selects a protected Custom Executor override; `flow executor configure --default` removes it. Resolution never uses the Workspace, `PATH`, a shell, the working directory or provider output.

Flow Agent probes the selected object once before durable productive Run reservation. An absent, unsafe, replaced, incompatible or unready object fails with stable diagnostics and creates no Run. A passing probe is advisory, not certification; every Tool execution still validates its enforcement receipt.

## One Tool invocation

```mermaid
sequenceDiagram
  participant F as Flow Agent
  participant S as Run store
  participant E as Executor
  participant B as Bubblewrap/seccomp
  participant T as Tool
  F->>F: Validate Tool call and compile canonical policy
  F->>S: Synchronize Tool intent
  F->>E: One bounded request
  E->>E: Validate framing, descriptors and policy
  alt Setup fails
    E-->>F: Stable typed pre-Tool error
    F->>S: Persist terminal failure
  else Boundary ready
    E->>B: Construct exact mounts and isolation
    B->>T: Start Tool and descendants
    T-->>E: Bounded output and exit
    E->>B: Terminate descendants and tear down
    E-->>F: Tool result and enforcement receipt
    F->>F: Validate receipt and exact policy digest
    F->>S: Persist terminal result and receipt
  end
```

One Executor process handles one invocation. Standard input carries one closed request; standard output carries one tagged canonical response plus final LF; standard error is bounded redacted diagnostics. There is no daemon, socket, pooled guest or remote transport.

The success receipt identifies the Executor/backend versions and exact platform, proves an active boundary and binds SHA-256 over the exact canonical policy bytes including their final LF. Flow Agent persists the canonical receipt with the terminal Tool attempt before publishing success. Custom Executors can lie, so schema validation is not third-party certification.

## Filesystem and runtime reads

Each `read_only_mounts` entry becomes an exact read-only mount and each `writable_mounts` entry an exact writable mount. Flow Agent opens each source without following links, verifies its identity and assigns a fixed inherited-descriptor slot. The request declares each slot and identity; all undeclared inherited descriptors are closed. Path replacement after validation therefore cannot redirect the mount.

Every Tool uses one runtime-read profile:

- `exact` is the default. It exposes only the bounded executable/interpreter/library objects advertised by the administrator-owned Executor readiness response.
- `host-system-read` is an explicit Agentic Engineer choice. It adds only the official Executor's fixed reviewed read-only Ubuntu system roots.

Flow selects the configured profile, pre-opens every advertised source without following links and binds each identity and Sandbox target into the resolved policy digest. The Executor cannot add a runtime path after that digest is fixed. Flow users, providers and Tools cannot change or escalate the profile. The official Ubuntu Executor is statically linked, so its own bootstrap does not require broad runtime reads; a dynamic official artifact fails readiness.

The backend uses stock Ubuntu Bubblewrap. Newer versions consume inherited descriptor mounts directly. For an older supported version, the outer Executor mounts `/proc/self/fd/<slot>` and starts a trusted inner `flow-executor` self-reexec that verifies the mounted device/inode identities before Tool execution. Missing support or mismatched identity fails closed. There is no bundled Bubblewrap, Landlock-only path, broad compatibility mount or unsandboxed fallback.

## Supported matrix

| Target | Official behavior |
|---|---|
| Ubuntu 24.04 x64 | Stock Bubblewrap namespaces and exact mounts plus seccomp; isolated Tool network namespace; deny all. |
| macOS | Productive Tool execution fails closed. Reconsider only after a post-M1.2 review proves supported controls that prevent or contain process creation and guarantee descendant teardown without private-API debt. |
| Windows and other targets | Productive Tool execution fails closed before Executor selection or Tool spawn. Custom Executors are not enabled. |

Positive Tool egress remains disabled. Provider traffic is not Tool egress.

## Reference architectures

The cited projects informed the boundary but are neither dependencies nor support promises.

### Pi Coding Agent

Pi's built-in Tools use the Pi process's host authority. Extensions can replace Tool operations with OS-sandbox or Gondolin integrations. Flow Agent adopts the narrow replaceable seam, but supplies a fail-closed default boundary instead of transferring isolation choice to an ordinary user.

### Codex CLI

Codex integrates approvals, permission profiles, command transformation, managed networking, lifecycle and several platform helpers. Flow Agent needs only its policy-aware one-shot Executor, Ubuntu backend, lifecycle and evidence. It does not add interactive approvals, PTYs, a proxy, a remote execution service, a bundled Sandbox binary or multiple official platform stacks.

This reduction keeps the product boundary auditable. The security-critical work remains exact capability translation, packaging, descendant cleanup and hostile black-box evidence, not the JSON transport itself.

## Evidence and quality goals

The canonical test matrix is in [`TESTING.md`](../../TESTING.md). It covers protocol framing, selection/readiness, exact mounts and replacement races, both runtime profiles, stock Bubblewrap compatibility, process descendants, deny-all networking, receipts, persistence/recovery, installation opt-out and fail-closed targets. Performance observations follow [`PERFORMANCE.md`](../../PERFORMANCE.md); no estimated timing, throughput or RSS value is a release gate.

## Pinned reference sources

Research snapshot: 2026-08-17.

- Pi Coding Agent, commit [`8720548`](https://github.com/earendil-works/pi/tree/87205484bf749c2140fef5d1bea68995d57e739c), MIT: [`README.md`, `security.md` and `rpc.md`](https://github.com/earendil-works/pi/tree/87205484bf749c2140fef5d1bea68995d57e739c/packages/coding-agent), the [`sandbox/index.ts` example](https://github.com/earendil-works/pi/tree/87205484bf749c2140fef5d1bea68995d57e739c/packages/coding-agent/examples/extensions/sandbox), and the [`gondolin/index.ts` example](https://github.com/earendil-works/pi/tree/87205484bf749c2140fef5d1bea68995d57e739c/packages/coding-agent/examples/extensions/gondolin).
- OpenAI Codex CLI, commit [`21cfd36`](https://github.com/openai/codex/tree/21cfd369efca2df70c904c580b2e7e2e3eddb3c3), Apache-2.0: [sandbox orchestration](https://github.com/openai/codex/tree/21cfd369efca2df70c904c580b2e7e2e3eddb3c3/codex-rs/core/src/sandboxing), [Linux backend](https://github.com/openai/codex/tree/21cfd369efca2df70c904c580b2e7e2e3eddb3c3/codex-rs/linux-sandbox), and [platform policy transformation](https://github.com/openai/codex/tree/21cfd369efca2df70c904c580b2e7e2e3eddb3c3/codex-rs/sandboxing/src).

Any later dependency requires the repository's normal licensing, security and supply-chain review.
