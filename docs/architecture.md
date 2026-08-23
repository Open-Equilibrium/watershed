# Current implementation architecture

These diagrams map the current Rust workspace and major Flow Agent responsibility paths. They do not depict the planned M1.2 boundary; see the [Flow Agent Executor architecture](concept/flow-agent-executor-architecture.md) for that target. Product topology is canonical in [`VISION.md`](../VISION.md); runtime behavior and storage contracts are canonical in [`PROTOCOL.md`](../PROTOCOL.md). Security and evidence remain in [`SECURITY.md`](../SECURITY.md) and [`TESTING.md`](../TESTING.md).

## Rust workspace crates

Arrows mean “depends on.” This graph includes current Cargo workspace members only.

```mermaid
flowchart TD
  C[flow-agent-cli] --> F[flow-agent-core]
  C --> S[core-script]
  C --> R[proto]
  F --> P[core-policy]
  F --> S
  F --> R
  P --> S
  P --> R
  S --> R
```

The package manifests are the executable dependency source of truth.

## Flow Agent runtime responsibilities

These are major control/data paths, not an exhaustive Rust module dependency graph or public embedding API.

### Execution paths

```mermaid
flowchart TD
  D[CLI dispatch] --> S[session]
  S --> P[fixture planning]
  P --> A[apply]
  A --> F[fixture effects and Tools]
  S --> R[M1.1 productive runtime]
  R --> O[OpenAI Codex provider]
  R --> Y[policy resolution]
  Y --> T[bounded direct Tool runner]
```

The fixture path is deterministic and in process. The current M1.1 productive Tool runner manages process stability but provides no OS-isolation boundary.

### Persistence and inputs

```mermaid
flowchart TD
  S[session] --> C[conversations]
  C --> W[conversation writer]
  W --> J[Event and context JSONL]
  W --> L[Live-event watermarks]
  C --> Q[Run Log, recovery and status]
  G[Workspace config and context] --> S
```

See the [Flow Agent V-Spec](concept/V-Spec_FlowAgent.html#architecture) for responsibility boundaries. Current module declarations live in [`flow-agent-core/src/runtime/mod.rs`](../flow-agent/flow-agent-core/src/runtime/mod.rs); CLI composition lives in [`flow-agent-cli/src/dispatch.rs`](../flow-agent/flow-agent-cli/src/dispatch.rs).
