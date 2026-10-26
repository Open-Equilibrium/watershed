# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.7 `flutter test` (debug JIT).
- All three workloads observed on 2026-10-26 at commit `8f2ff48` plus the example change committed with this record; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries in 2,050,826 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples. A second run of the example writes the file first, so building it stays out of the measured process | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as 169 to 181 events, the largest carrying 3,056 bytes of merged text. Maximum resident set size 8.3 MB, peak memory footprint 7.0 MB.

Long Trajectory, core: the first read from the session file takes most of each sample (10.5 ms p50); the tree and both Trajectories over the compact index take about 3.3 ms together, and the follow-up read of new entries about 0.06 ms. The process peaks at 50.9 MB resident (17.2 MB footprint). Earlier records built the session file in the measured process and are not comparable, nor are runs that read the stored entries from one agent response.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":20.293,"events":178,"delta_events":173,"largest_delta_bytes":2448}
{"sample":1,"elapsed_ms":13.784,"events":181,"delta_events":176,"largest_delta_bytes":2704}
{"sample":2,"elapsed_ms":11.398,"events":176,"delta_events":171,"largest_delta_bytes":2496}
{"sample":3,"elapsed_ms":11.367,"events":173,"delta_events":168,"largest_delta_bytes":2608}
{"sample":4,"elapsed_ms":11.311,"events":175,"delta_events":170,"largest_delta_bytes":2592}
{"sample":5,"elapsed_ms":11.268,"events":172,"delta_events":167,"largest_delta_bytes":2624}
{"sample":6,"elapsed_ms":11.381,"events":175,"delta_events":170,"largest_delta_bytes":2592}
{"sample":7,"elapsed_ms":11.285,"events":175,"delta_events":170,"largest_delta_bytes":2576}
{"sample":8,"elapsed_ms":11.205,"events":176,"delta_events":171,"largest_delta_bytes":2624}
{"sample":9,"elapsed_ms":11.250,"events":177,"delta_events":172,"largest_delta_bytes":2560}
{"sample":10,"elapsed_ms":11.162,"events":172,"delta_events":167,"largest_delta_bytes":2592}
{"sample":11,"elapsed_ms":11.223,"events":176,"delta_events":171,"largest_delta_bytes":2576}
{"sample":12,"elapsed_ms":11.267,"events":173,"delta_events":168,"largest_delta_bytes":2496}
{"sample":13,"elapsed_ms":10.990,"events":172,"delta_events":167,"largest_delta_bytes":2624}
{"sample":14,"elapsed_ms":11.204,"events":175,"delta_events":170,"largest_delta_bytes":2624}
{"sample":15,"elapsed_ms":11.315,"events":175,"delta_events":170,"largest_delta_bytes":2592}
{"sample":16,"elapsed_ms":11.129,"events":176,"delta_events":171,"largest_delta_bytes":2624}
{"sample":17,"elapsed_ms":11.026,"events":169,"delta_events":164,"largest_delta_bytes":2496}
{"sample":18,"elapsed_ms":11.306,"events":175,"delta_events":170,"largest_delta_bytes":2480}
{"sample":19,"elapsed_ms":11.329,"events":174,"delta_events":169,"largest_delta_bytes":3056}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":11.268,"p95_ms":13.784,"max_ms":20.293}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":16.407,"tree_ms":0.676,"trajectory_ms":2.275,"fork_trajectory_ms":1.919,"total_ms":21.278,"update_ms":0.068,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":12.415,"tree_ms":0.471,"trajectory_ms":1.708,"fork_trajectory_ms":1.547,"total_ms":16.140,"update_ms":0.061,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":10.558,"tree_ms":0.415,"trajectory_ms":1.520,"fork_trajectory_ms":1.379,"total_ms":13.872,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":10.433,"tree_ms":0.415,"trajectory_ms":1.513,"fork_trajectory_ms":1.383,"total_ms":13.745,"update_ms":0.061,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":10.396,"tree_ms":0.408,"trajectory_ms":1.499,"fork_trajectory_ms":1.367,"total_ms":13.670,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":10.438,"tree_ms":0.416,"trajectory_ms":1.490,"fork_trajectory_ms":1.372,"total_ms":13.715,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":10.462,"tree_ms":0.414,"trajectory_ms":1.515,"fork_trajectory_ms":1.399,"total_ms":13.789,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":10.472,"tree_ms":0.428,"trajectory_ms":1.520,"fork_trajectory_ms":1.377,"total_ms":13.798,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":10.357,"tree_ms":0.404,"trajectory_ms":1.491,"fork_trajectory_ms":1.380,"total_ms":13.632,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":10.416,"tree_ms":0.425,"trajectory_ms":1.497,"fork_trajectory_ms":1.381,"total_ms":13.719,"update_ms":0.051,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":10.449,"tree_ms":0.417,"trajectory_ms":1.509,"fork_trajectory_ms":1.380,"total_ms":13.756,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":10.944,"tree_ms":0.461,"trajectory_ms":1.618,"fork_trajectory_ms":1.487,"total_ms":14.510,"update_ms":0.080,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":11.021,"tree_ms":0.415,"trajectory_ms":1.492,"fork_trajectory_ms":1.372,"total_ms":14.300,"update_ms":0.063,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":10.491,"tree_ms":0.419,"trajectory_ms":1.510,"fork_trajectory_ms":1.420,"total_ms":13.840,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":10.351,"tree_ms":0.406,"trajectory_ms":1.492,"fork_trajectory_ms":1.387,"total_ms":13.637,"update_ms":0.057,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":10.534,"tree_ms":0.415,"trajectory_ms":1.501,"fork_trajectory_ms":1.379,"total_ms":13.829,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":10.330,"tree_ms":0.415,"trajectory_ms":1.496,"fork_trajectory_ms":1.389,"total_ms":13.631,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":10.481,"tree_ms":0.406,"trajectory_ms":1.508,"fork_trajectory_ms":1.376,"total_ms":13.772,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":10.351,"tree_ms":0.434,"trajectory_ms":1.549,"fork_trajectory_ms":1.409,"total_ms":13.742,"update_ms":0.061,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":10.438,"tree_ms":0.412,"trajectory_ms":1.497,"fork_trajectory_ms":1.396,"total_ms":13.743,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","file_bytes":2050826,"samples":20,"p50_ms":13.756,"p95_ms":16.140,"max_ms":21.278}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.31,"p95_ms":2.806,"max_ms":3.198}
{"operation":"turnGroups","samples":20,"p50_ms":0.203,"p95_ms":0.825,"max_ms":1.134}
{"operation":"segments","samples":20,"p50_ms":0.051,"p95_ms":0.094,"max_ms":0.28}
{"operation":"inRange","samples":20,"p50_ms":0.039,"p95_ms":0.083,"max_ms":0.105}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.117,"p95_ms":0.243,"max_ms":0.276}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.78,"p95_ms":0.926,"max_ms":1.014}
```
