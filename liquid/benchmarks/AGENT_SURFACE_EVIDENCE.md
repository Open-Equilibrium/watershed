# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- Observed on 2026-10-24 at commit `867f444`; one run each, after two earlier runs with comparable timings.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries, 2,050,797 bytes of `get_entries` data; per sample a fresh session reads the whole history, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as about 175 events, the largest carrying 3104 bytes of merged text. Maximum resident set size 7.8 MB, peak memory footprint 6.6 MB.

Long Trajectory, core: the first history read takes most of each sample; the tree and both Trajectories over the compact index take about 2.6 ms together (p50), and the follow-up read of new entries about 0.07 ms. The process peaks at 64.4 MB resident (45.8 MB footprint). Runs before the incremental history re-read the whole session in one long-lived session and are not comparable.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":12.551,"events":180,"delta_events":174,"largest_delta_bytes":2496}
{"sample":1,"elapsed_ms":12.440,"events":178,"delta_events":172,"largest_delta_bytes":2512}
{"sample":2,"elapsed_ms":12.215,"events":177,"delta_events":171,"largest_delta_bytes":2544}
{"sample":3,"elapsed_ms":12.155,"events":180,"delta_events":174,"largest_delta_bytes":2528}
{"sample":4,"elapsed_ms":12.439,"events":176,"delta_events":170,"largest_delta_bytes":2640}
{"sample":5,"elapsed_ms":12.276,"events":182,"delta_events":176,"largest_delta_bytes":2608}
{"sample":6,"elapsed_ms":12.388,"events":179,"delta_events":173,"largest_delta_bytes":2640}
{"sample":7,"elapsed_ms":12.350,"events":176,"delta_events":170,"largest_delta_bytes":2496}
{"sample":8,"elapsed_ms":12.399,"events":182,"delta_events":176,"largest_delta_bytes":3104}
{"sample":9,"elapsed_ms":12.431,"events":179,"delta_events":173,"largest_delta_bytes":2480}
{"sample":10,"elapsed_ms":12.426,"events":178,"delta_events":172,"largest_delta_bytes":2512}
{"sample":11,"elapsed_ms":12.447,"events":177,"delta_events":171,"largest_delta_bytes":2624}
{"sample":12,"elapsed_ms":12.248,"events":174,"delta_events":168,"largest_delta_bytes":3088}
{"sample":13,"elapsed_ms":12.374,"events":173,"delta_events":167,"largest_delta_bytes":2496}
{"sample":14,"elapsed_ms":12.311,"events":174,"delta_events":168,"largest_delta_bytes":2496}
{"sample":15,"elapsed_ms":12.455,"events":175,"delta_events":169,"largest_delta_bytes":2592}
{"sample":16,"elapsed_ms":12.400,"events":176,"delta_events":170,"largest_delta_bytes":2496}
{"sample":17,"elapsed_ms":12.566,"events":177,"delta_events":171,"largest_delta_bytes":2496}
{"sample":18,"elapsed_ms":12.337,"events":174,"delta_events":168,"largest_delta_bytes":2480}
{"sample":19,"elapsed_ms":12.374,"events":177,"delta_events":171,"largest_delta_bytes":2480}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":12.388,"p95_ms":12.551,"max_ms":12.566}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":12.031,"tree_ms":0.533,"trajectory_ms":1.307,"fork_trajectory_ms":1.077,"total_ms":14.948,"update_ms":0.077,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":11.217,"tree_ms":0.450,"trajectory_ms":1.147,"fork_trajectory_ms":1.027,"total_ms":13.841,"update_ms":0.070,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":10.544,"tree_ms":0.444,"trajectory_ms":1.143,"fork_trajectory_ms":1.020,"total_ms":13.152,"update_ms":0.066,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":10.653,"tree_ms":0.453,"trajectory_ms":1.159,"fork_trajectory_ms":1.025,"total_ms":13.290,"update_ms":0.068,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":10.354,"tree_ms":0.449,"trajectory_ms":1.148,"fork_trajectory_ms":1.030,"total_ms":12.981,"update_ms":0.065,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":10.461,"tree_ms":0.450,"trajectory_ms":1.149,"fork_trajectory_ms":1.019,"total_ms":13.080,"update_ms":0.071,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":10.421,"tree_ms":0.440,"trajectory_ms":1.140,"fork_trajectory_ms":1.025,"total_ms":13.025,"update_ms":0.068,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":10.528,"tree_ms":0.439,"trajectory_ms":1.140,"fork_trajectory_ms":1.023,"total_ms":13.131,"update_ms":0.066,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":10.671,"tree_ms":0.454,"trajectory_ms":1.149,"fork_trajectory_ms":1.023,"total_ms":13.297,"update_ms":0.069,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":10.551,"tree_ms":0.438,"trajectory_ms":1.138,"fork_trajectory_ms":1.019,"total_ms":13.146,"update_ms":0.070,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":10.554,"tree_ms":0.441,"trajectory_ms":1.137,"fork_trajectory_ms":1.022,"total_ms":13.155,"update_ms":0.068,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":10.428,"tree_ms":0.439,"trajectory_ms":1.138,"fork_trajectory_ms":1.025,"total_ms":13.030,"update_ms":0.067,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":10.608,"tree_ms":0.454,"trajectory_ms":1.150,"fork_trajectory_ms":1.031,"total_ms":13.243,"update_ms":0.078,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":10.676,"tree_ms":0.443,"trajectory_ms":1.132,"fork_trajectory_ms":1.013,"total_ms":13.264,"update_ms":0.076,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":10.412,"tree_ms":0.440,"trajectory_ms":1.144,"fork_trajectory_ms":1.028,"total_ms":13.023,"update_ms":0.067,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":10.592,"tree_ms":0.442,"trajectory_ms":1.140,"fork_trajectory_ms":1.019,"total_ms":13.193,"update_ms":0.067,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":10.405,"tree_ms":0.442,"trajectory_ms":1.133,"fork_trajectory_ms":1.023,"total_ms":13.003,"update_ms":0.069,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":10.711,"tree_ms":0.451,"trajectory_ms":1.159,"fork_trajectory_ms":1.029,"total_ms":13.350,"update_ms":0.075,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":10.579,"tree_ms":0.445,"trajectory_ms":1.147,"fork_trajectory_ms":1.040,"total_ms":13.211,"update_ms":0.065,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":10.525,"tree_ms":0.449,"trajectory_ms":1.151,"fork_trajectory_ms":1.022,"total_ms":13.146,"update_ms":0.069,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","entries_bytes":2050797,"samples":20,"p50_ms":13.152,"p95_ms":13.841,"max_ms":14.948}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.244,"p95_ms":3.146,"max_ms":4.635}
{"operation":"turnGroups","samples":20,"p50_ms":0.173,"p95_ms":0.843,"max_ms":1.182}
{"operation":"segments","samples":20,"p50_ms":0.051,"p95_ms":0.256,"max_ms":0.324}
{"operation":"inRange","samples":20,"p50_ms":0.039,"p95_ms":0.085,"max_ms":0.109}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.118,"p95_ms":0.284,"max_ms":0.31}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.768,"p95_ms":0.93,"max_ms":0.978}
```
