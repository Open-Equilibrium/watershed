# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- All three workloads observed on 2026-10-25 at commit `b1c3234`; one run each, after two earlier runs with comparable timings.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries in 2,050,826 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as about 175 events, the largest carrying 4096 bytes of merged text. Maximum resident set size 8.3 MB, peak memory footprint 6.9 MB.

Long Trajectory, core: the first read from the session file takes most of each sample (11.0 ms p50); the tree and both Trajectories over the compact index take about 2.7 ms together, and the follow-up read of new entries about 0.06 ms. The process peaks at 62.0 MB resident (45.9 MB footprint). Keeping each entry's usage in the entry had raised the footprint to 83 MB; the history now sums usage while it reads. Runs that read the stored entries from one agent response are not comparable.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":12.624,"events":179,"delta_events":173,"largest_delta_bytes":2480}
{"sample":1,"elapsed_ms":12.351,"events":176,"delta_events":170,"largest_delta_bytes":2960}
{"sample":2,"elapsed_ms":12.561,"events":180,"delta_events":174,"largest_delta_bytes":2480}
{"sample":3,"elapsed_ms":12.640,"events":178,"delta_events":172,"largest_delta_bytes":3392}
{"sample":4,"elapsed_ms":12.258,"events":181,"delta_events":175,"largest_delta_bytes":3152}
{"sample":5,"elapsed_ms":12.565,"events":177,"delta_events":171,"largest_delta_bytes":2576}
{"sample":6,"elapsed_ms":12.555,"events":174,"delta_events":168,"largest_delta_bytes":2464}
{"sample":7,"elapsed_ms":12.513,"events":177,"delta_events":171,"largest_delta_bytes":2560}
{"sample":8,"elapsed_ms":12.594,"events":178,"delta_events":172,"largest_delta_bytes":2560}
{"sample":9,"elapsed_ms":12.446,"events":176,"delta_events":170,"largest_delta_bytes":2560}
{"sample":10,"elapsed_ms":12.412,"events":176,"delta_events":170,"largest_delta_bytes":2560}
{"sample":11,"elapsed_ms":12.562,"events":175,"delta_events":169,"largest_delta_bytes":2576}
{"sample":12,"elapsed_ms":12.584,"events":178,"delta_events":172,"largest_delta_bytes":2576}
{"sample":13,"elapsed_ms":12.285,"events":176,"delta_events":170,"largest_delta_bytes":2576}
{"sample":14,"elapsed_ms":12.559,"events":180,"delta_events":174,"largest_delta_bytes":3040}
{"sample":15,"elapsed_ms":12.402,"events":180,"delta_events":174,"largest_delta_bytes":2560}
{"sample":16,"elapsed_ms":12.453,"events":172,"delta_events":166,"largest_delta_bytes":4080}
{"sample":17,"elapsed_ms":12.623,"events":177,"delta_events":171,"largest_delta_bytes":2480}
{"sample":18,"elapsed_ms":12.434,"events":180,"delta_events":174,"largest_delta_bytes":2896}
{"sample":19,"elapsed_ms":12.694,"events":171,"delta_events":165,"largest_delta_bytes":4096}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":12.555,"p95_ms":12.640,"max_ms":12.694}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":11.415,"tree_ms":0.490,"trajectory_ms":1.241,"fork_trajectory_ms":1.125,"total_ms":14.272,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":11.028,"tree_ms":0.434,"trajectory_ms":1.147,"fork_trajectory_ms":1.067,"total_ms":13.675,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":11.069,"tree_ms":0.434,"trajectory_ms":1.162,"fork_trajectory_ms":1.074,"total_ms":13.738,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":11.124,"tree_ms":0.444,"trajectory_ms":1.207,"fork_trajectory_ms":1.078,"total_ms":13.852,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":11.184,"tree_ms":0.444,"trajectory_ms":1.209,"fork_trajectory_ms":1.094,"total_ms":13.931,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":10.985,"tree_ms":0.448,"trajectory_ms":1.198,"fork_trajectory_ms":1.073,"total_ms":13.704,"update_ms":0.064,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":11.008,"tree_ms":0.441,"trajectory_ms":1.197,"fork_trajectory_ms":1.080,"total_ms":13.726,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":10.808,"tree_ms":0.441,"trajectory_ms":1.168,"fork_trajectory_ms":1.083,"total_ms":13.499,"update_ms":0.052,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":10.909,"tree_ms":0.438,"trajectory_ms":1.198,"fork_trajectory_ms":1.058,"total_ms":13.603,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":10.981,"tree_ms":0.434,"trajectory_ms":1.139,"fork_trajectory_ms":1.058,"total_ms":13.612,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":10.824,"tree_ms":0.427,"trajectory_ms":1.187,"fork_trajectory_ms":1.060,"total_ms":13.498,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":10.956,"tree_ms":0.426,"trajectory_ms":1.184,"fork_trajectory_ms":1.059,"total_ms":13.625,"update_ms":0.053,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":11.085,"tree_ms":0.417,"trajectory_ms":1.130,"fork_trajectory_ms":1.052,"total_ms":13.684,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":11.044,"tree_ms":0.443,"trajectory_ms":1.194,"fork_trajectory_ms":1.083,"total_ms":13.764,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":10.779,"tree_ms":0.424,"trajectory_ms":1.168,"fork_trajectory_ms":1.062,"total_ms":13.433,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":10.904,"tree_ms":0.428,"trajectory_ms":1.179,"fork_trajectory_ms":1.065,"total_ms":13.576,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":10.864,"tree_ms":0.427,"trajectory_ms":1.175,"fork_trajectory_ms":1.065,"total_ms":13.531,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":10.781,"tree_ms":0.419,"trajectory_ms":1.123,"fork_trajectory_ms":1.055,"total_ms":13.379,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":10.798,"tree_ms":0.430,"trajectory_ms":1.130,"fork_trajectory_ms":1.054,"total_ms":13.412,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":10.830,"tree_ms":0.420,"trajectory_ms":1.188,"fork_trajectory_ms":1.043,"total_ms":13.481,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","file_bytes":2050826,"samples":20,"p50_ms":13.612,"p95_ms":13.931,"max_ms":14.272}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.35,"p95_ms":3.137,"max_ms":5.137}
{"operation":"turnGroups","samples":20,"p50_ms":0.176,"p95_ms":0.792,"max_ms":0.817}
{"operation":"segments","samples":20,"p50_ms":0.051,"p95_ms":0.212,"max_ms":0.283}
{"operation":"inRange","samples":20,"p50_ms":0.04,"p95_ms":0.086,"max_ms":0.111}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.12,"p95_ms":0.268,"max_ms":0.281}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.793,"p95_ms":0.969,"max_ms":1.008}
```
