# Performance

Performance is an architectural quality KPI, not a set of estimated pass/fail numbers. Design for responsive interaction, bounded resource use, useful concurrency and proportional work. Prefer streaming, bounded queues, incremental processing, narrow data movement and explicit ownership; avoid repeated parsing, copies, scans, process starts, blocking and serialization when a simpler boundary removes them.

Timing, throughput and RSS measurements are observational regression evidence. They do not fail a build or block a release because an absolute value or percentage changed. A numeric performance SLO may become binding only after the maintainer approves a measured user requirement, reference environment, stable workload and enforcement method through the decision flow.

This policy does not relax hard security, correctness, capacity, liveness or durability bounds. Deadlines that terminate stalled external work, byte/count limits that prevent unbounded resource use, checked arithmetic, protocol limits, coverage and evidence-integrity checks remain enforced by their canonical contracts.

## Review convention

Every product change is reviewed for its performance effects alongside security, stability, functionality, simplicity and maintainability. Review the relevant call paths and consumers rather than one file in isolation. Evidence must answer:

- Does the architecture keep work and retained state bounded by the product contract?
- Does cost scale with useful input, or does the change introduce avoidable repeated work, copying, polling, contention or global serialization?
- Are concurrency and cancellation paths free of avoidable blocking and leaks?
- Does a simpler design remove work or state without weakening correctness or isolation?
- Do fixed-workload observations show a material regression, and if so, is its architectural cause understood?

Prefer an architectural simplification over a local micro-optimization. Never weaken isolation, cleanup, correctness or a hard capacity boundary to improve a measurement. If a material regression has no clear maintainable fix, retain the evidence and record the tradeoff through the decision flow.

## Evidence convention

Performance evidence uses a fixed, documented workload and records the environment, exact inputs and exclusions, raw observations and useful aggregates such as p50, p95, maximum and RSS growth. Values remain unadjusted; do not subtract estimated work or normalize away an observed cost. Compare like-for-like runs and keep enough history to identify trends.

CI may fail when the workload, lifecycle, schema, measurement integrity, report completeness or artifact retention is invalid. It must not compare timing, throughput or RSS observations with an estimated threshold. Command and test timeouts remain deadlock guards, not performance claims.

## Flow Agent

Flow Agent should add little overhead around provider and Tool work, remain responsive under useful concurrency and stream histories and artifacts without retaining complete data unnecessarily. Roots and Subflows share the hard process-wide capacity limit below; provider and Tool processes, network latency and caller-owned buffers are outside Flow Agent performance evidence.

### ADR-0068 safety envelope

These hard capacity and liveness limits are not performance thresholds and are not multiplied into one promised maximum workload:

- Flow tree depth: 16 levels, root-inclusive.
- Direct fan-out: 32 Subflow references per Flow definition.
- Cumulative invocations: 512 per Run, including failed starts.
- Live invocations: 32 process-wide across all sessions.
- Canonical events: 155,750 per Run.
- Canonical storage: 320 KiB per event including LF; 16 MiB per event or context-manifest segment; 352 MiB total event data; 48 MiB total context-manifest data; 16 MiB per immutable object; 131,072 immutable objects; 5,216 MiB object data; and 5.5 GiB per complete Run bundle.

The event sizing model, segment rotation, storage admission and exact boundary tests are canonical in `PROTOCOL.md`, `TESTING.md` and the [M1.1 limits matrix](flow-agent/benchmarks/M1_1_BUDGETS.md). Dormant on-disk histories and Run bundles have no retention quota and are never deleted by these limits.

### M1.1 evidence

The [M1.1 limits matrix](flow-agent/benchmarks/M1_1_BUDGETS.md) owns every fixed functional limit and the selected observational workloads. The Ubuntu 24.04 x64 evidence suite runs each warmup and sample in a fresh child, records unadjusted timing and Linux RSS observations, validates that its fixed RSS fixture is detectable and retains the `m11-performance-evidence` artifact. It has no timing or RSS pass/fail comparison.

### M1.2 Executor evidence

The one-shot Executor and Sandbox architecture is canonical in `PROTOCOL.md`. The [M1.2 startup evidence](flow-agent/benchmarks/M1_2_STARTUP_EVIDENCE.md) records one fixed direct Tool invocation per fresh child, with independent unadjusted runner and Tool-runtime distributions. Executor performance is reviewed against the one-shot design and isolation boundary; Custom Executor performance is administrator-owned.

## Meta-Harness

Control and monitoring should feel immediate when they are not waiting on a model, Tool or network. Keep configuration resolution incremental, event normalization bounded and AgentPulse ingestion proportional to active sessions. Add fixed observational workloads with each implemented surface that introduces a meaningful cost center.

## Liquid

Local interaction should feel immediate, shared workspaces should remain responsive under useful human and agent concurrency, and organization scale should come from workspace sharding and scope-filtered subscriptions rather than global fan-out. Rendering, mutation, history, query, sync and App Runtime paths should avoid whole-workspace recomputation and unbounded retained state. Add representative fixed observational workloads as those surfaces are implemented; select a hard SLO only through the evidence rule above.
