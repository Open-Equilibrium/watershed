# Performance Targets (NFR)

Non-functional targets. These are **falsifiable budgets**: every target must have a test or benchmark (see `TESTING.md`). Targets are split into product-level goals and implementation budgets so the MVP can fail fast on the actual bottleneck.

Rationale for the Rust core: low per-agent memory footprint, true multi-core parallelism, and a post-M1 Wasmtime plugin isolation path — properties a Node runtime cannot meet at the required scale.

## Loop Agent

Product target:
- **10 parallel top-level loops** on a laptop-class device with at least 4 logical cores, 16 GiB RAM and SSD storage. Their roots and subloops share the process-wide limit of 32 live invocations. Model/provider processes, tool processes, network latency and caller-owned output buffers are excluded.

### D-068 safety envelope

These hard limits always apply and are not multiplied into one promised workload:

- Loop tree depth: 16 levels, root = level 1.
- Direct fan-out: 32 subloop references per Loop definition.
- Cumulative invocations: 512 per session, root-inclusive. Every start counts, including repeated definitions, retries and failed starts.
- Live invocations: 32 process-wide across all sessions. A started non-terminal root or subloop counts while running or waiting for model, tool or child work; queued, terminal and fully paused work does not.
- Canonical events: 155,750 per session across all segments, resumes, errors and future event families.
- Canonical storage: 320 KiB per event including LF, 16 MiB per event segment or context-manifest segment, 48 MiB total event data, 48 MiB total context-manifest data, 16 MiB per immutable object and 5.5 GiB for the complete logical session bundle. Object data may use at most 5,520 MiB, reserving the other 112 MiB for the two JSONL streams and metadata. The former 10 MiB event-stream limit is removed.

The 155,750-event sizing model is `2 session + 1,024 Loop lifecycle + 1,024 phase + 102,400 model-cycle + 100 non-model-turn + 51,200 tool lifecycle`. It assumes 25,600 model cycles and 25,600 tool calls. Four model-cycle events mean one enclosing `step.started`/`step.completed` pair plus `message.delta`/`message.completed`; a non-model turn needs only the step pair. These are capacity assumptions, not independent product limits.

M1 plans the complete deterministic stream before side effects. A future dynamic planner must keep 20 event slots free until clean termination: one active tool/message terminal event, one `step.completed`, up to 16 `loop.failed` events, one `error` and one `session.failed`.

### Storage evidence and profiles

This private, untracked calibration ran on 2026-07-18. The Codex scan covered the reference rollout and every recursively referenced subagent rollout, deduplicated by resolved file path: 538 physical JSONL files. Each physical JSONL line is one record; sizes are UTF-8 bytes including LF. Percentiles sort record sizes and select index `floor(p * (n - 1))`. Codex records size storage only; they are not Loop protocol events and do not change the 155,750-event semantic cap.

| 2026-07-18 sample | Records | Bytes | p50 | p90 | p99 | Max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Codex reference root `019f5a0b-ad56-7053-b2c9-f933d1e921e8` | 63,757 | 102.65 MiB | 708 B | 2,431 B | 22,755 B | 132,899 B |
| Complete recursive Codex tree, 538 rollout files | 4,202,848 | 4.799 GiB | 671 B | 747 B | 11,816 B | 286,174 B |
| Checked-in Loop expected JSONL, 9 files | 129 | 36.24 KiB | 280 B | 359 B | 403 B | 412 B |

The independent Loop sample averages 287.67 B/event, projecting to 42.73 MiB at 155,750 events. The recursive Codex sample includes all discovered subagent rollout files; it is the basis for the rounded 5.5 GiB bundle ceiling. Session-owned hash-addressed immutable objects preserve referenced canonical context bytes without duplicating equal content. Self-containment guarantees durable replay of canonical history even if workspace sources disappear; it does not recreate an external provider, tool, mutable environment or undeclared side effect.

Sixteen MiB is a per-segment rotation boundary, not a session or RAM limit. Rotation occurs before a record would cross it and never splits a record. At the 48 MiB event-data ceiling this normally yields three event segments and at most four when unsplittable event boundaries leave slack; arbitrary unsplit manifest records need at most five. This keeps sequential reads and recovery bounded without the extra file/flush overhead of materially smaller segments or the larger per-read burst of 32/64 MiB segments.

The representative payload distribution is 90% of events at 768 B, the next 9% at 12 KiB, the next 0.9% at 96 KiB and the final 0.1% at 320 KiB. This is a distribution of complete canonical event sizes, not `max payload / 2`. Exactly 16,000 events occupy 48,152,576 B (45.92 MiB).

Synthetic event-storage/replay workload:

- Representative: 10 sessions, each with 32 cumulative invocations and 16,000 total events under that payload distribution; aggregate 320 invocations, 160,000 events and 459.22 MiB. The 32-live process cap still applies.
- Full-cap: one 155,750-event session with the independent 288 B/event profile (42.78 MiB).
- Stability: ten sessions each reach 155,750 events with the 288 B/event profile. This does not simultaneously maximize payload sizes, registry size, live model contexts or tool output.

M1 implementation budgets (ADR-0049):
- FSM transition overhead p95 <= 1 ms per event, excluding model and tool work.
- Local no-op tool dispatch overhead p95 <= 50 ms per run, excluding the tool's own runtime.
- Memory overhead <= 10 MiB per active top-level loop before LLM/tool payloads, including one unique resolved registry closure shared by that Loop and its subloops (ADR-0067).
- Log/event append latency p95 <= 5 ms per event for the `hello-loop` canonical serialization and local append path.
- Live-notification attempt p95 <= 50 ms after a successful append, covering the bounded high-watermark update and non-blocking wake-up attempt but excluding caller-owned replay and transport (ADR-0059, ADR-0062).
- `message.delta`/`tool.progress` micro-batches wait no longer than 25 ms before append; semantic or terminal events close a pending batch immediately (ADR-0059).
- Concurrency smoke: 10 fixture top-level loops complete without harness-level deadlock or unbounded memory growth; 10 near-limit closures remain within the same 100 MiB aggregate RSS budget.
- Initial full-session replay <= 10 s and full-session inspection <= 15 s at the D-068 event cap, with <= 256 MiB RSS growth.
- Incremental tail read p95 <= 100 ms for one newly committed event up to 320 KiB, with <= 64 MiB retained-reader RSS growth.
- The representative ten-session and ten-full-cap event-storage/replay gates each complete <= 120 s; they are not end-to-end runtime gates.

Timing and RSS gates run in release mode, one performance test process at a time, on the fixed `ubuntu-24.04` x64 CI image. Other operating systems run functional boundary tests; performance claims require the reference hardware class above.

Tool runs are bounded/headless; the harness itself must not be the bottleneck when local inference is fast.

The event budgets measure individual events, not averages of batch averages; ordering and durability semantics are canonical in `PROTOCOL.md`.

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
2. **Single shared workspace:** **250 concurrent active actors** (humans + agents) with p95 mutation→ack < 250 ms and p95 mutation→peer-visible < 1 s.
3. **Organization scale (design-for):** **1,000 users + 5,000 agents** across many workspaces via workspace sharding and **Block- and scope-filtered event subscriptions** (clients receive only subscribed state — no workspace-wide broadcast), holding the tier-2 latencies. Depends on the sync/conflict model (D-035); it is a history/storage/event-model constraint, **not** an MVP gate.
4. **Throughput budget:** ≥ 1,000 mutations/s sustained per workspace node, including action-history append.

The binding constraint at scale is event fan-out, not mutation processing; Block- and scope-filtered subscriptions are therefore a design assumption, not an optimization.

Implementation budgets to define before M3:
- Block update dispatch latency.
- Page/View render/update latency for representative content and Arrange mode.
- Workspace query latency for common PowerBar lookups and CLI/API reads.
- Action-history append latency per workspace mutation.
- Diff calculation latency for common Block/Page changes.
- Revert latency for recent actions.
- Memory per Block/View/session card.

## Notes

- "ms range" applies to Watershed's own overhead (event dispatch, state updates, rendering pipeline), not to externally-bound latency (LLM, network, remote tools).
- Product targets are not enough by themselves; each milestone must convert its target into lower-level benchmark budgets before implementation starts.
- Targets are reviewed per milestone; changes go through an ADR (see `AGENTS.md`).
