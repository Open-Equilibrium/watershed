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
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as about 180 events, the largest carrying 3296 bytes of merged text. Maximum resident set size 7.9 MB, peak memory footprint 6.7 MB.

Long Trajectory, core: the history read takes most of each sample. The process peaks at 99.5 MB resident (74.9 MB footprint), between 63 and 101 MB across runs. Each read parses the whole session into a `serde_json::Value`, and the Trajectory tab reads again after each stored change; reading only new entries would bound this, and typed records would shrink it.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":24.691,"events":178,"delta_events":172,"largest_delta_bytes":2432}
{"sample":1,"elapsed_ms":15.549,"events":176,"delta_events":170,"largest_delta_bytes":2496}
{"sample":2,"elapsed_ms":12.796,"events":178,"delta_events":172,"largest_delta_bytes":2592}
{"sample":3,"elapsed_ms":12.240,"events":177,"delta_events":171,"largest_delta_bytes":3296}
{"sample":4,"elapsed_ms":12.208,"events":175,"delta_events":169,"largest_delta_bytes":2608}
{"sample":5,"elapsed_ms":12.404,"events":178,"delta_events":172,"largest_delta_bytes":3296}
{"sample":6,"elapsed_ms":12.304,"events":175,"delta_events":169,"largest_delta_bytes":2592}
{"sample":7,"elapsed_ms":12.239,"events":175,"delta_events":169,"largest_delta_bytes":2464}
{"sample":8,"elapsed_ms":12.310,"events":178,"delta_events":172,"largest_delta_bytes":2480}
{"sample":9,"elapsed_ms":12.306,"events":175,"delta_events":169,"largest_delta_bytes":2480}
{"sample":10,"elapsed_ms":12.379,"events":174,"delta_events":168,"largest_delta_bytes":2480}
{"sample":11,"elapsed_ms":12.213,"events":172,"delta_events":166,"largest_delta_bytes":2464}
{"sample":12,"elapsed_ms":12.430,"events":176,"delta_events":170,"largest_delta_bytes":2480}
{"sample":13,"elapsed_ms":12.243,"events":181,"delta_events":175,"largest_delta_bytes":2480}
{"sample":14,"elapsed_ms":12.432,"events":180,"delta_events":174,"largest_delta_bytes":2992}
{"sample":15,"elapsed_ms":12.289,"events":176,"delta_events":170,"largest_delta_bytes":2592}
{"sample":16,"elapsed_ms":12.259,"events":180,"delta_events":174,"largest_delta_bytes":2624}
{"sample":17,"elapsed_ms":12.111,"events":179,"delta_events":173,"largest_delta_bytes":2496}
{"sample":18,"elapsed_ms":12.196,"events":179,"delta_events":173,"largest_delta_bytes":2592}
{"sample":19,"elapsed_ms":12.348,"events":179,"delta_events":173,"largest_delta_bytes":2480}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":12.304,"p95_ms":15.549,"max_ms":24.691}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":13.642,"tree_ms":2.143,"trajectory_ms":2.407,"fork_trajectory_ms":1.934,"total_ms":20.127,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":10.058,"tree_ms":1.950,"trajectory_ms":2.316,"fork_trajectory_ms":1.949,"total_ms":16.274,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":9.534,"tree_ms":1.827,"trajectory_ms":2.270,"fork_trajectory_ms":1.971,"total_ms":15.602,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":9.590,"tree_ms":1.887,"trajectory_ms":2.299,"fork_trajectory_ms":1.974,"total_ms":15.751,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":9.538,"tree_ms":1.797,"trajectory_ms":2.146,"fork_trajectory_ms":1.967,"total_ms":15.448,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":9.941,"tree_ms":1.764,"trajectory_ms":2.215,"fork_trajectory_ms":1.961,"total_ms":15.881,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":9.522,"tree_ms":1.773,"trajectory_ms":2.141,"fork_trajectory_ms":1.941,"total_ms":15.377,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":9.615,"tree_ms":1.878,"trajectory_ms":2.232,"fork_trajectory_ms":1.941,"total_ms":15.666,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":9.558,"tree_ms":1.813,"trajectory_ms":2.190,"fork_trajectory_ms":1.942,"total_ms":15.504,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":9.456,"tree_ms":1.855,"trajectory_ms":2.215,"fork_trajectory_ms":1.952,"total_ms":15.479,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":9.444,"tree_ms":1.784,"trajectory_ms":2.128,"fork_trajectory_ms":1.938,"total_ms":15.294,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":9.684,"tree_ms":1.825,"trajectory_ms":2.229,"fork_trajectory_ms":1.972,"total_ms":15.709,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":9.593,"tree_ms":1.774,"trajectory_ms":2.214,"fork_trajectory_ms":1.956,"total_ms":15.537,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":9.496,"tree_ms":1.792,"trajectory_ms":2.198,"fork_trajectory_ms":1.935,"total_ms":15.421,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":9.257,"tree_ms":1.758,"trajectory_ms":2.184,"fork_trajectory_ms":1.950,"total_ms":15.150,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":9.644,"tree_ms":1.793,"trajectory_ms":2.181,"fork_trajectory_ms":1.962,"total_ms":15.579,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":9.653,"tree_ms":1.766,"trajectory_ms":2.195,"fork_trajectory_ms":1.936,"total_ms":15.550,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":9.437,"tree_ms":1.795,"trajectory_ms":2.177,"fork_trajectory_ms":1.945,"total_ms":15.354,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":9.515,"tree_ms":1.759,"trajectory_ms":2.151,"fork_trajectory_ms":1.936,"total_ms":15.360,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":9.663,"tree_ms":1.858,"trajectory_ms":2.236,"fork_trajectory_ms":1.966,"total_ms":15.723,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","entries_bytes":2050797,"samples":20,"p50_ms":15.537,"p95_ms":16.274,"max_ms":20.127}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.285,"p95_ms":2.958,"max_ms":4.497}
{"operation":"turnGroups","samples":20,"p50_ms":0.182,"p95_ms":0.805,"max_ms":1.114}
{"operation":"segments","samples":20,"p50_ms":0.05,"p95_ms":0.117,"max_ms":0.24}
{"operation":"inRange","samples":20,"p50_ms":0.039,"p95_ms":0.09,"max_ms":0.113}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.099,"p95_ms":0.24,"max_ms":0.271}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.625,"p95_ms":0.768,"max_ms":0.879}
```
