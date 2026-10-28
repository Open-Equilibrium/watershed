# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- All three workloads observed on 2026-10-26 at commit `733887b`; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries in 2,050,826 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples. A second run of the example writes the file first, so building it stays out of the measured process | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as 173 to 184 events, the largest carrying 3,120 bytes of merged text. Maximum resident set size 9.3 MB, peak memory footprint 7.9 MB.

Long Trajectory, core: the first read from the session file takes most of each sample (10.7 ms p50); the tree and both Trajectories over the compact index take about 3.4 ms together, and the follow-up read of new entries about 0.06 ms. The process's peak memory footprint is 17.2 MB. `/usr/bin/time` reports 50.9 MB maximum resident set size, but that counts the waited-for writer run too, so only the footprint describes the measured process. Earlier records built the session file in the measured process and are not comparable, nor are runs that read the stored entries from one agent response.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":19.985,"events":180,"delta_events":175,"largest_delta_bytes":2448}
{"sample":1,"elapsed_ms":13.484,"events":178,"delta_events":173,"largest_delta_bytes":2608}
{"sample":2,"elapsed_ms":11.284,"events":175,"delta_events":170,"largest_delta_bytes":2480}
{"sample":3,"elapsed_ms":11.238,"events":176,"delta_events":171,"largest_delta_bytes":2576}
{"sample":4,"elapsed_ms":11.190,"events":175,"delta_events":170,"largest_delta_bytes":2464}
{"sample":5,"elapsed_ms":11.433,"events":184,"delta_events":179,"largest_delta_bytes":2544}
{"sample":6,"elapsed_ms":11.531,"events":175,"delta_events":170,"largest_delta_bytes":2896}
{"sample":7,"elapsed_ms":11.416,"events":174,"delta_events":169,"largest_delta_bytes":2480}
{"sample":8,"elapsed_ms":11.330,"events":175,"delta_events":170,"largest_delta_bytes":2480}
{"sample":9,"elapsed_ms":11.320,"events":180,"delta_events":175,"largest_delta_bytes":2576}
{"sample":10,"elapsed_ms":11.460,"events":174,"delta_events":169,"largest_delta_bytes":2576}
{"sample":11,"elapsed_ms":11.360,"events":176,"delta_events":171,"largest_delta_bytes":2608}
{"sample":12,"elapsed_ms":11.426,"events":177,"delta_events":172,"largest_delta_bytes":2480}
{"sample":13,"elapsed_ms":11.308,"events":178,"delta_events":173,"largest_delta_bytes":2992}
{"sample":14,"elapsed_ms":11.429,"events":174,"delta_events":169,"largest_delta_bytes":3120}
{"sample":15,"elapsed_ms":11.233,"events":176,"delta_events":171,"largest_delta_bytes":2560}
{"sample":16,"elapsed_ms":11.401,"events":180,"delta_events":175,"largest_delta_bytes":2464}
{"sample":17,"elapsed_ms":11.218,"events":174,"delta_events":169,"largest_delta_bytes":2512}
{"sample":18,"elapsed_ms":11.394,"events":175,"delta_events":170,"largest_delta_bytes":2592}
{"sample":19,"elapsed_ms":11.498,"events":173,"delta_events":168,"largest_delta_bytes":2464}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":11.394,"p95_ms":13.484,"max_ms":19.985}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":16.439,"tree_ms":0.678,"trajectory_ms":2.201,"fork_trajectory_ms":1.890,"total_ms":21.209,"update_ms":0.070,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":12.573,"tree_ms":0.467,"trajectory_ms":1.671,"fork_trajectory_ms":1.476,"total_ms":16.188,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":10.854,"tree_ms":0.421,"trajectory_ms":1.528,"fork_trajectory_ms":1.399,"total_ms":14.202,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":10.688,"tree_ms":0.421,"trajectory_ms":1.535,"fork_trajectory_ms":1.411,"total_ms":14.055,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":10.658,"tree_ms":0.418,"trajectory_ms":1.550,"fork_trajectory_ms":1.417,"total_ms":14.044,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":10.828,"tree_ms":0.432,"trajectory_ms":1.525,"fork_trajectory_ms":1.405,"total_ms":14.191,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":10.690,"tree_ms":0.422,"trajectory_ms":1.552,"fork_trajectory_ms":1.414,"total_ms":14.078,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":10.636,"tree_ms":0.420,"trajectory_ms":1.558,"fork_trajectory_ms":1.419,"total_ms":14.032,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":10.721,"tree_ms":0.426,"trajectory_ms":1.567,"fork_trajectory_ms":1.432,"total_ms":14.146,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":10.584,"tree_ms":0.419,"trajectory_ms":1.519,"fork_trajectory_ms":1.391,"total_ms":13.912,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":10.607,"tree_ms":0.423,"trajectory_ms":1.550,"fork_trajectory_ms":1.415,"total_ms":13.996,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":10.715,"tree_ms":0.424,"trajectory_ms":1.576,"fork_trajectory_ms":1.432,"total_ms":14.147,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":10.564,"tree_ms":0.418,"trajectory_ms":1.525,"fork_trajectory_ms":1.398,"total_ms":13.905,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":10.612,"tree_ms":0.421,"trajectory_ms":1.534,"fork_trajectory_ms":1.392,"total_ms":13.959,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":10.634,"tree_ms":0.425,"trajectory_ms":1.551,"fork_trajectory_ms":1.410,"total_ms":14.020,"update_ms":0.053,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":10.643,"tree_ms":0.426,"trajectory_ms":1.560,"fork_trajectory_ms":1.411,"total_ms":14.040,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":10.739,"tree_ms":0.427,"trajectory_ms":1.534,"fork_trajectory_ms":1.404,"total_ms":14.103,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":10.667,"tree_ms":0.437,"trajectory_ms":1.549,"fork_trajectory_ms":1.441,"total_ms":14.094,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":10.761,"tree_ms":0.415,"trajectory_ms":1.541,"fork_trajectory_ms":1.430,"total_ms":14.147,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":10.777,"tree_ms":0.429,"trajectory_ms":1.582,"fork_trajectory_ms":1.422,"total_ms":14.210,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","file_bytes":2050826,"samples":20,"p50_ms":14.078,"p95_ms":16.188,"max_ms":21.209}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.322,"p95_ms":3.253,"max_ms":4.542}
{"operation":"turnGroups","samples":20,"p50_ms":0.199,"p95_ms":0.786,"max_ms":1.135}
{"operation":"segments","samples":20,"p50_ms":0.051,"p95_ms":0.267,"max_ms":0.312}
{"operation":"inRange","samples":20,"p50_ms":0.039,"p95_ms":0.097,"max_ms":0.111}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.118,"p95_ms":0.291,"max_ms":0.366}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.791,"p95_ms":0.964,"max_ms":1.014}
```
