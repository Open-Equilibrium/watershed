# Current implementation architecture

These diagrams map the Rust workspace and major Flow Agent responsibility paths. The [Flow Agent Executor architecture](concept/flow-agent-executor-architecture.md) explains the M1.2 isolation design. Product topology is canonical in [`VISION.md`](../VISION.md); runtime behavior and storage contracts are canonical in [`PROTOCOL.md`](../PROTOCOL.md). Security and evidence remain in [`SECURITY.md`](../SECURITY.md) and [`TESTING.md`](../TESTING.md).

## Rust workspace crates

Arrows mean “depends on.” This graph includes current Cargo workspace members only.

```mermaid
flowchart TD
  C[flow-agent-cli] --> F[flow-agent-core]
  C --> S[core-script]
  C --> R[proto]
  E[flow-agent-executor] --> P[core-policy]
  E[flow-agent-executor] --> R
  F --> P[core-policy]
  F --> S
  F --> R
  P --> S
  P --> R
  S --> R
```

The package manifests are the executable dependency source of truth. `flow-agent-executor` is the companion binary launched through the protocol, not a library dependency of Flow Agent core.

## Flow Agent runtime responsibilities

These are major control/data paths, not an exhaustive Rust module dependency graph or public embedding API.

### Execution paths

```mermaid
flowchart TD
  D[CLI dispatch] --> S[session]
  S --> P[fixture planning]
  P --> A[apply]
  A --> F[fixture effects and Tools]
  S --> R[productive runtime]
  R --> O[OpenAI Codex provider]
  R --> Y[policy resolution]
  Y --> E[one-shot Executor client]
  E --> X[flow-executor]
  X --> B[Ubuntu Bubblewrap and seccomp]
  B --> T[Tool process and descendants]
```

The Fixture path is deterministic and in process and makes no OS-isolation claim. Productive Tool execution uses the one-shot Executor contract; official productive targets without the Ubuntu boundary fail closed.

### Persistence and inputs

```mermaid
flowchart TD
  S[session] --> C[conversations]
  C --> W[conversation writer]
  W --> J[Event and context JSONL]
  W --> L[Live-event watermarks]
  C --> Q[Run Log, recovery, enforcement receipt and status]
  G[Global config and registry] --> S
  A[Global and Workspace AGENTS.md] --> S
  X[Execution Workspace] --> S
```

See the [Flow Agent V-Spec](concept/V-Spec_FlowAgent.html#architecture) for responsibility boundaries. Current module declarations live in [`flow-agent-core/src/runtime/mod.rs`](../flow-agent/flow-agent-core/src/runtime/mod.rs); CLI composition lives in [`flow-agent-cli/src/dispatch.rs`](../flow-agent/flow-agent-cli/src/dispatch.rs).

## Configuration boundary

Flow Agent resolves technical configuration only from `FLOW_AGENT_HOME`, defaulting to `~/.flow` on Unix and `%USERPROFILE%\.flow` on Windows. Its `config.yaml` selects a registry only beneath that global home. Workspace `.flow` files and registries are never implicit inputs. Global-home and harness-start Workspace `AGENTS.md` files are loaded separately as ordered instructions and cannot alter technical authority. Executor selection is a separate protected administrator setting: the default derives the sibling `flow-executor`, while a Custom Executor requires an absolute override. [`PROTOCOL.md`](../PROTOCOL.md) owns both contracts.
