# Performance Targets (NFR)

Non-functional targets. These are **falsifiable budgets**: every target must have a test or benchmark (see `TESTING.md`). Targets are split into product-level goals and implementation budgets so the MVP can fail fast on the actual bottleneck.

Rationale for the Rust core: low per-agent memory footprint, true multi-core parallelism, and Wasmtime plugin isolation — properties a Node runtime cannot meet at the required scale.

## Loop Agent

Product target:
- **10 parallel top-level loops** on a single device, each *including its own subloops/sub-agents* (i.e. 10 orchestrating loops, not 10 leaf agents).

M1 implementation budgets (ADR-0049):
- FSM transition overhead p95 <= 1 ms per event, excluding model and tool work.
- Local no-op tool dispatch overhead p95 <= 50 ms per run, excluding the tool's own runtime.
- Memory overhead <= 10 MiB per active top-level loop before LLM/tool payloads.
- Log/event append latency p95 <= 5 ms per event for the `hello-loop` JSONL stream/session log path.
- Concurrency smoke: 10 fixture top-level loops complete without harness-level deadlock or unbounded memory growth.

Tool runs are bounded/headless; the harness itself must not be the bottleneck when local inference is fast.

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

1. **Local UI:** p95 < 100 ms from user action to acknowledged workspace mutation; representative dashboards hold a 60 fps render budget.
2. **Single shared workspace:** **250 concurrent active actors** (humans + agents) with p95 mutation→ack < 250 ms and p95 mutation→peer-visible < 1 s.
3. **Organization scale (design-for):** **1,000 users + 5,000 agents** across many workspaces via workspace sharding and **view-scoped event subscriptions** (clients receive only events for views/scopes they subscribe to — no workspace-wide broadcast), holding the tier-2 latencies. Depends on the sync/collaboration model (D-035) and remote deployment; a design constraint on the history/storage/event model, **not** an MVP gate.
4. **Throughput budget:** ≥ 1,000 mutations/s sustained per workspace node, including action-history append.

The binding constraint at scale is event fan-out, not mutation processing; view-scoped subscriptions are therefore a design assumption, not an optimization.

Implementation budgets to define before M3:
- Component update dispatch latency.
- Dashboard/view render/update latency for representative views.
- Workspace query latency for common PowerBar lookups and CLI/API reads.
- Action-history append latency per workspace mutation.
- Diff calculation latency for common component/dashboard changes.
- Revert latency for recent actions.
- Memory per component/view/session card.

## Notes

- "ms range" applies to Watershed's own overhead (event dispatch, state updates, rendering pipeline), not to externally-bound latency (LLM, network, remote tools).
- Product targets are not enough by themselves; each milestone must convert its target into lower-level benchmark budgets before implementation starts.
- Targets are reviewed per milestone; changes go through an ADR (see `AGENTS.md`).
