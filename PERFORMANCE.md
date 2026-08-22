# Performance Targets (NFR)

Non-functional targets. These are **evidence-based, falsifiable guidelines**: every target must have a test or benchmark (see `TESTING.md`). Targets are split into product-level goals and implementation budgets so the MVP can fail fast on the actual bottleneck.

A performance-gate exceedance blocks release until it is dispositioned; the selected number is not immutable. The maintainer classifies the exceedance as small or large. A small exceedance requires re-evaluating the numeric target and its evidence. A large exceedance requires re-evaluating the workload, architecture and limit structure. Optimize only when a small, clear and maintainable fix is feasible; otherwise add an open design decision before changing the implementation or target. This policy does not relax functional, safety or capacity boundaries explicitly defined as hard limits or exact `F`-row contracts.

Rationale for the Rust core: low per-agent memory footprint, true multi-core parallelism, and a post-M1 Wasmtime plugin isolation path — properties a Node runtime cannot meet at the required scale.

## Flow Agent

Flow Agent budgets govern work while Flow Agent is executing. Dormant on-disk conversation histories and run bundles have no count, age or byte quota and never trigger retention or deletion. Complete-history validation uses the per-command temporary index selected by [CV-03, CV-04, CV-09 and CV-13–CV-14](flow-agent/benchmarks/M1_1_BUDGETS.md#conversations-and-run-logs): it has bounded scan, memory and scratch, uses checked arithmetic and performs at most O(n log n) work. Other command reads likewise keep scan, projection, memory, output and latency streaming or bounded by an approved budget.

Product target:
- **10 parallel top-level flows** on a laptop-class device with at least 4 logical cores, 16 GiB RAM and SSD storage. Their roots and subflows share the process-wide limit of 32 live invocations. Model/provider processes, tool processes, network latency and caller-owned output buffers are excluded.

### ADR-0068 safety envelope

These hard limits always apply and are not multiplied into one promised workload:

- Flow tree depth: 16 levels, root = level 1.
- Direct fan-out: 32 subflow references per Flow definition.
- Cumulative invocations: 512 per Run, root-inclusive. Every start counts, including repeated definitions, loop iterations and failed starts.
- Live invocations: 32 process-wide across all sessions. A started non-terminal root or subflow counts while running or waiting for model, tool or child work; queued, terminal and fully paused work does not.
- Canonical events: 155,750 per Run across all segments, resumes, errors and future event families.
- Canonical storage: 320 KiB per event including LF, 16 MiB per event segment or context-manifest segment, 352 MiB total event data, 48 MiB total context-manifest data, 16 MiB per immutable object, at most 131,072 immutable objects per Run and 5.5 GiB for the complete Run bundle. Object data may use at most 5,216 MiB, reserving the other 416 MiB for the two JSONL streams and metadata. The former 10 MiB event-stream limit is removed.

The 155,750-event sizing model is `2 session + 1,024 Flow lifecycle + 1,024 Phase lifecycle + 102,400 provider-message lifecycle + 51,200 Tool lifecycle + 100 terminal/error reserve`. It represents 51,200 message pairs and 25,600 Tool lifecycle pairs. The 100-event term is sizing slack for terminal orchestration, not an independently addressable product limit. These are capacity assumptions, not independent product limits.

Sixteen MiB is a per-segment rotation boundary, not a session or RAM limit. Rotation occurs before a record would cross it and never splits a record. An event stream uses at most 22 segments; record-boundary slack can make that count bind before the 352 MiB byte ceiling. Arbitrary unsplit manifest records use at most five segments. This keeps sequential reads and recovery bounded without the extra file/flush overhead of materially smaller segments or the larger per-read burst of 32/64 MiB segments.

The representative payload distribution is 90% of events at 768 B, the next 9% at 12 KiB, the next 0.9% at 96 KiB and the final 0.1% at 320 KiB. This is a distribution of complete canonical event sizes, not `max payload / 2`. Exactly 16,000 events occupy 48,152,576 B (45.92 MiB).

Synthetic event-storage/replay workload:

- Representative: 10 sessions, each with 32 cumulative invocations and 16,000 total events under that payload distribution; aggregate 320 invocations, 160,000 events and 459.22 MiB. The 32-live process cap still applies.
- Full-cap: one 155,750-event session with the synthetic 288 B/event profile (42.78 MiB).
- Stability: ten sessions each reach 155,750 events with the 288 B/event profile. This does not simultaneously maximize payload sizes, registry size, live model contexts or tool output.

M1 implementation budgets (ADR-0049):
- FSM transition overhead p95 <= 1 ms per event, excluding model and tool work.
- Local no-op tool dispatch overhead p95 <= 50 ms per run, excluding the tool's own runtime.
- Memory overhead <= 11 MiB per active top-level flow before LLM/tool payloads, including one unique resolved registry closure shared by that Flow and its subflows (ADR-0067, ADR-0150).
- Log/event append latency p95 <= 5 ms per event for the `hello-flow` canonical serialization and local append path.
- Live-notification attempt p95 <= 50 ms after a successful append, covering the bounded high-watermark update and non-blocking wake-up attempt but excluding caller-owned replay and transport (ADR-0059, ADR-0062).
- `message.delta`/`tool.progress` micro-batches wait no longer than 25 ms before append; semantic or terminal events close a pending batch immediately (ADR-0059).
- Concurrency smoke: 10 fixture top-level flows complete without harness-level deadlock or unbounded memory growth; 10 near-limit closures remain within the same 110 MiB aggregate RSS budget.
- Callback-streaming full-Run replay uses the workload, latency and peak-RSS evidence in [CV-17/CV-18](flow-agent/benchmarks/M1_1_BUDGETS.md#conversations-and-run-logs). Full-Run inspection with a retained maximum object inventory completes <= 15 s with <= 256 MiB RSS growth.
- Incremental tail read p95 <= 100 ms for one newly committed event up to 320 KiB, with <= 64 MiB retained-reader RSS growth.
- The representative ten-session and ten-full-cap event-storage/replay gates each complete <= 120 s; they are not end-to-end runtime gates.

Timing and RSS gates run in release mode, one performance test process at a time, on the fixed `ubuntu-24.04` x64 CI image. Other operating systems run functional boundary tests; performance claims require the reference hardware class above.

ADR-0150 recalibrated the per-Flow guideline from 20 fresh isolated release-mode samples in [CI run 32571620281](https://github.com/Open-Equilibrium/watershed/actions/runs/32571620281) at commit `7679ebc`. Peak RSS growth ranged from 10,559,488 to 10,756,096 bytes, with a 10,645,299.2-byte mean. Eleven MiB is the smallest whole-MiB ceiling above the observed maximum and leaves 778,240 bytes, or 7.24%, measurement headroom. The 110 MiB aggregate gate preserves the exact tenfold relationship for 10 concurrent top-level Flows.

ADR-0151 recalibrated two latency guidelines from independent 30-sample Ubuntu release runs at commit `188318b`: [push run 32572626735](https://github.com/Open-Equilibrium/watershed/actions/runs/32572626735) measured migration/replay p95 of 1.192 s/12.354 s, and [pull-request run 32572629127](https://github.com/Open-Equilibrium/watershed/actions/runs/32572629127) measured 1.191 s/12.309 s. CV-12 migration is therefore 1.25 s and CV-17 replay is 13 s, leaving 4.83% and 5.23% above the larger observed p95 values. Workloads, samples, functional and capacity boundaries, and RSS guidelines are unchanged.

Tool runs are bounded/headless; the harness itself must not be the bottleneck when local inference is fast.

The event budgets measure individual events, not averages of batch averages; ordering and durability semantics are canonical in `PROTOCOL.md`.

ADR-0107 selects the E3 evidence scope, balanced bundle A and finite evidence matrix; ADR-0123 adds the bounded status-summary contract and reuses its fixed status-page workload. The [M1.1 budget matrix](flow-agent/benchmarks/M1_1_BUDGETS.md) is the single source for every selected numeric cap, deadline and optimized gate, its exact counting rule, functional boundary proof, named performance workload, measurement-validation fixture and justified exclusion. Its evidence must pass before M1.1 product behavior begins. Under ADR-0106, every optimized workload sample and warmup runs in a fresh child process. Linux peak RSS growth is `post-workload VmHWM - pre-workload VmRSS`, which includes rather than subtracts recorded lifetime-high-water slack; retained growth is `post-workload VmRSS - pre-workload VmRSS`. Reports remain unadjusted. Because `flow-tool-result-v0` stores each non-inline stdout/stderr stream as one immutable object, the per-stream collector caps in [TR-01/TR-02](flow-agent/benchmarks/M1_1_BUDGETS.md#tool-runner) remain below the existing per-object limit.

## Meta-Harness

Product target:
- **Reactivity in the millisecond range** for control/monitoring actions (start/stop/steer/config), **excluding** network/internet latency and excluding LLM/tool execution time.

Implementation budgets to define before M2:
- Session-registry update latency.
- Adapter event-normalization latency.
- AgentPulse metric-sample ingestion latency.
- Config-resolution latency for shared building blocks resolved to target CLIs.

## Liquid

Product targets (tiered; each tier gets its own benchmark — ADR-0014):

1. **Local UI:** p95 < 100 ms from user action to acknowledged workspace mutation; representative Pages and Arrange mode hold a 60 fps render budget.
2. **Single shared workspace:** **250 concurrent active actors** (humans + agents) with p95 mutation→ack < 250 ms and p95 committed mutation→replica-visible < 1 s while connected to the central Sync Server.
3. **Organization scale (design-for):** **1,000 users + 5,000 agents** across many Workspaces via Workspace sharding and scope-filtered query/event subscriptions, holding the tier-2 latencies. Subscription filtering limits live fan-out; it does not change the authorized Workspace as the replication unit. Depends on D-035 and is **not** an MVP gate.
4. **Throughput budget:** ≥ 1,000 mutations/s sustained per workspace node, including action-history append.

The binding constraint at scale is event fan-out, not mutation processing; scope-filtered live subscriptions are therefore a design assumption, not an optimization.

Implementation budgets to define before M3:
- Block update dispatch latency.
- Page/View render/update latency for representative content and Arrange mode.
- Workspace query latency for common PowerBar lookups and CLI/API reads.
- Action-history append latency per workspace mutation.
- Diff calculation latency for common Block/Page changes.
- Revert latency for recent actions.
- Memory per Block/View/session card.
- App Runtime startup, action-dispatch, memory and CPU/time-limit enforcement.
- Sync Server commit acknowledgement and replica catch-up latency after reconnect.

## Notes

- "ms range" applies to Watershed's own overhead (event dispatch, state updates, rendering pipeline), not to externally-bound latency (LLM, network, remote tools).
- Product targets are not enough by themselves; each milestone must convert its target into lower-level benchmark budgets before implementation starts.
- Targets are reviewed per milestone; changes go through an ADR (see `AGENTS.md`).
