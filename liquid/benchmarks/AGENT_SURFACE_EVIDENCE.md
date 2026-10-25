# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- Token streaming and the Dart workload observed on 2026-10-24 at commit `867f444`, the core Trajectory workload on 2026-10-25 at commit `1021f53`; one run each, after two earlier runs with comparable timings.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries in 2,050,826 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as about 175 events, the largest carrying 3104 bytes of merged text. Maximum resident set size 7.8 MB, peak memory footprint 6.6 MB.

Long Trajectory, core: the first read from the session file takes most of each sample (9.7 ms p50); the tree and both Trajectories over the compact index take about 2.5 ms together, and the follow-up read of new entries about 0.06 ms. The process peaks at 61.7 MB resident (45.8 MB footprint). Earlier runs read the stored entries from one agent response and are not comparable.

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
{"sample":0,"read_ms":9.957,"tree_ms":0.471,"trajectory_ms":1.215,"fork_trajectory_ms":1.078,"total_ms":12.721,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":9.825,"tree_ms":0.409,"trajectory_ms":1.105,"fork_trajectory_ms":1.002,"total_ms":12.342,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":9.702,"tree_ms":0.403,"trajectory_ms":1.106,"fork_trajectory_ms":1.002,"total_ms":12.213,"update_ms":0.062,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":9.728,"tree_ms":0.405,"trajectory_ms":1.106,"fork_trajectory_ms":1.004,"total_ms":12.242,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":9.690,"tree_ms":0.412,"trajectory_ms":1.147,"fork_trajectory_ms":1.013,"total_ms":12.262,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":9.785,"tree_ms":0.403,"trajectory_ms":1.123,"fork_trajectory_ms":1.017,"total_ms":12.327,"update_ms":0.053,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":9.760,"tree_ms":0.410,"trajectory_ms":1.098,"fork_trajectory_ms":0.998,"total_ms":12.267,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":9.775,"tree_ms":0.404,"trajectory_ms":1.109,"fork_trajectory_ms":1.005,"total_ms":12.293,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":9.728,"tree_ms":0.402,"trajectory_ms":1.097,"fork_trajectory_ms":0.995,"total_ms":12.222,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":9.702,"tree_ms":0.408,"trajectory_ms":1.104,"fork_trajectory_ms":1.004,"total_ms":12.218,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":9.651,"tree_ms":0.405,"trajectory_ms":1.102,"fork_trajectory_ms":0.996,"total_ms":12.154,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":9.723,"tree_ms":0.411,"trajectory_ms":1.124,"fork_trajectory_ms":1.007,"total_ms":12.264,"update_ms":0.052,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":9.710,"tree_ms":0.413,"trajectory_ms":1.111,"fork_trajectory_ms":1.006,"total_ms":12.240,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":9.707,"tree_ms":0.402,"trajectory_ms":1.099,"fork_trajectory_ms":1.002,"total_ms":12.210,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":9.693,"tree_ms":0.405,"trajectory_ms":1.109,"fork_trajectory_ms":1.006,"total_ms":12.213,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":9.704,"tree_ms":0.408,"trajectory_ms":1.114,"fork_trajectory_ms":1.009,"total_ms":12.234,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":9.666,"tree_ms":0.402,"trajectory_ms":1.115,"fork_trajectory_ms":1.007,"total_ms":12.190,"update_ms":0.053,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":9.644,"tree_ms":0.405,"trajectory_ms":1.109,"fork_trajectory_ms":1.007,"total_ms":12.165,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":9.628,"tree_ms":0.404,"trajectory_ms":1.111,"fork_trajectory_ms":1.005,"total_ms":12.147,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":9.775,"tree_ms":0.414,"trajectory_ms":1.106,"fork_trajectory_ms":1.002,"total_ms":12.296,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","file_bytes":2050826,"samples":20,"p50_ms":12.234,"p95_ms":12.342,"max_ms":12.721}
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
