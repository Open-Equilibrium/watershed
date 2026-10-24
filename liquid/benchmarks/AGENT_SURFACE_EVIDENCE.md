# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- Observed on 2026-10-24; one run each, after two earlier runs with comparable timings.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries, 2,050,797 bytes of `get_entries` data; per sample one history read, the tree and the Trajectories of the main branch and the last fork; 20 samples in one session | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The same session shape: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as about 180 events, the largest carrying 3072 bytes of merged text. Maximum resident set size 7.8 MB, peak memory footprint 6.5 MB.

Long Trajectory, core: the history read takes most of each sample. The process peaks at 100.5 MB resident (76.3 MB footprint), between 63 and 100 MB across runs. Each read parses the whole session into a `serde_json::Value`, and the Trajectory tab reads again after each stored change; reading only new entries would bound this, and typed records would shrink it.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":12.855,"events":182,"delta_events":176,"largest_delta_bytes":2480}
{"sample":1,"elapsed_ms":12.592,"events":185,"delta_events":179,"largest_delta_bytes":2624}
{"sample":2,"elapsed_ms":12.507,"events":179,"delta_events":173,"largest_delta_bytes":2528}
{"sample":3,"elapsed_ms":12.595,"events":183,"delta_events":177,"largest_delta_bytes":2608}
{"sample":4,"elapsed_ms":12.419,"events":179,"delta_events":173,"largest_delta_bytes":3040}
{"sample":5,"elapsed_ms":12.396,"events":181,"delta_events":175,"largest_delta_bytes":2560}
{"sample":6,"elapsed_ms":12.345,"events":179,"delta_events":173,"largest_delta_bytes":2608}
{"sample":7,"elapsed_ms":12.312,"events":182,"delta_events":176,"largest_delta_bytes":3056}
{"sample":8,"elapsed_ms":12.357,"events":176,"delta_events":170,"largest_delta_bytes":2464}
{"sample":9,"elapsed_ms":12.232,"events":181,"delta_events":175,"largest_delta_bytes":3056}
{"sample":10,"elapsed_ms":12.519,"events":178,"delta_events":172,"largest_delta_bytes":2480}
{"sample":11,"elapsed_ms":12.489,"events":179,"delta_events":173,"largest_delta_bytes":3072}
{"sample":12,"elapsed_ms":12.348,"events":181,"delta_events":175,"largest_delta_bytes":2512}
{"sample":13,"elapsed_ms":12.478,"events":180,"delta_events":174,"largest_delta_bytes":2608}
{"sample":14,"elapsed_ms":12.560,"events":178,"delta_events":172,"largest_delta_bytes":2512}
{"sample":15,"elapsed_ms":12.270,"events":178,"delta_events":172,"largest_delta_bytes":2512}
{"sample":16,"elapsed_ms":12.469,"events":179,"delta_events":173,"largest_delta_bytes":2608}
{"sample":17,"elapsed_ms":12.274,"events":178,"delta_events":172,"largest_delta_bytes":2800}
{"sample":18,"elapsed_ms":12.349,"events":180,"delta_events":174,"largest_delta_bytes":2496}
{"sample":19,"elapsed_ms":12.424,"events":180,"delta_events":174,"largest_delta_bytes":3056}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":12.419,"p95_ms":12.595,"max_ms":12.855}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":13.404,"tree_ms":2.000,"trajectory_ms":2.239,"fork_trajectory_ms":1.873,"total_ms":19.516,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":9.915,"tree_ms":1.821,"trajectory_ms":2.094,"fork_trajectory_ms":1.844,"total_ms":15.674,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":9.656,"tree_ms":1.877,"trajectory_ms":2.071,"fork_trajectory_ms":1.809,"total_ms":15.414,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":9.426,"tree_ms":1.842,"trajectory_ms":2.104,"fork_trajectory_ms":1.830,"total_ms":15.202,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":9.526,"tree_ms":1.796,"trajectory_ms":2.040,"fork_trajectory_ms":1.826,"total_ms":15.188,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":9.531,"tree_ms":1.835,"trajectory_ms":2.064,"fork_trajectory_ms":1.820,"total_ms":15.250,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":9.491,"tree_ms":1.783,"trajectory_ms":2.020,"fork_trajectory_ms":1.776,"total_ms":15.071,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":9.508,"tree_ms":1.816,"trajectory_ms":2.075,"fork_trajectory_ms":1.842,"total_ms":15.241,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":9.278,"tree_ms":1.771,"trajectory_ms":2.043,"fork_trajectory_ms":1.810,"total_ms":14.902,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":9.469,"tree_ms":1.800,"trajectory_ms":2.076,"fork_trajectory_ms":1.802,"total_ms":15.147,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":9.370,"tree_ms":1.737,"trajectory_ms":1.979,"fork_trajectory_ms":1.769,"total_ms":14.855,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":9.668,"tree_ms":1.762,"trajectory_ms":2.046,"fork_trajectory_ms":1.802,"total_ms":15.278,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":9.554,"tree_ms":1.758,"trajectory_ms":2.041,"fork_trajectory_ms":1.859,"total_ms":15.212,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":9.563,"tree_ms":1.786,"trajectory_ms":2.052,"fork_trajectory_ms":1.817,"total_ms":15.218,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":9.648,"tree_ms":1.764,"trajectory_ms":2.027,"fork_trajectory_ms":1.831,"total_ms":15.270,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":9.604,"tree_ms":1.798,"trajectory_ms":2.095,"fork_trajectory_ms":1.868,"total_ms":15.364,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":9.567,"tree_ms":1.752,"trajectory_ms":1.971,"fork_trajectory_ms":1.801,"total_ms":15.090,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":9.650,"tree_ms":1.798,"trajectory_ms":2.124,"fork_trajectory_ms":1.840,"total_ms":15.413,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":9.701,"tree_ms":1.790,"trajectory_ms":2.004,"fork_trajectory_ms":1.809,"total_ms":15.304,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":9.606,"tree_ms":1.783,"trajectory_ms":2.111,"fork_trajectory_ms":1.868,"total_ms":15.368,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","entries_bytes":2050797,"samples":20,"p50_ms":15.241,"p95_ms":15.674,"max_ms":19.516}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.209,"p95_ms":2.772,"max_ms":4.234}
{"operation":"turnGroups","samples":20,"p50_ms":0.158,"p95_ms":0.678,"max_ms":0.773}
{"operation":"segments","samples":20,"p50_ms":0.05,"p95_ms":0.296,"max_ms":0.622}
{"operation":"inRange","samples":20,"p50_ms":0.04,"p95_ms":0.094,"max_ms":0.108}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.117,"p95_ms":0.255,"max_ms":0.281}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.772,"p95_ms":0.95,"max_ms":1.028}
```
