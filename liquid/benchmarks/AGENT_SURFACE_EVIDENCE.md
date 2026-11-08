# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT).
- Token streaming observed on 2026-10-31 at commit `2b34840`; both Trajectory workloads on 2026-11-08 at `0a3e9fce`; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of seven entries (prompt, response with a tool call, tool result, final response and three timing entries) and eight forks of ten turns: 7,560 entries in 1,827,596 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples. A second run of the example writes the file first, so building it stays out of the measured process | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 7,630 tree nodes and 4,000 records on the main branch; the late fork shares 3,552 steps, the early fork 444; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: the 20,000 deltas arrive as 1 text event of 320,000 bytes, 6 events in all. Maximum resident set size 9.9 MB, peak memory footprint 8.4 MB. The record at `7781c3f`, whose forwarder also drained the channel after the window, counted 2 text events; earlier records before the 50 ms window counted 173 to 184 events; neither is comparable.

Long Trajectory, core: the first read from the session file takes most of each sample (10.3 ms p50); the tree and both Trajectories over the compact index take about 3.7 ms together, and the follow-up read of new entries about 0.06 ms. The process's peak memory footprint is 15.1 MB. `/usr/bin/time` reports 44.9 MB maximum resident set size, but that counts the waited-for writer run too, so only the footprint describes the measured process. Records before `0a3e9fce` stored an approval entry per turn and are not comparable.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 3,556 against 40 steps.

### Raw output

Token streaming:

```json
{"sample":0,"elapsed_ms":19.612,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":1,"elapsed_ms":13.004,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":2,"elapsed_ms":10.913,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":3,"elapsed_ms":10.817,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":4,"elapsed_ms":10.846,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":5,"elapsed_ms":10.682,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":6,"elapsed_ms":10.914,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":7,"elapsed_ms":10.645,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":8,"elapsed_ms":10.747,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":9,"elapsed_ms":10.668,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":10,"elapsed_ms":10.679,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":11,"elapsed_ms":10.877,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":12,"elapsed_ms":10.894,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":13,"elapsed_ms":10.967,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":14,"elapsed_ms":10.789,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":15,"elapsed_ms":10.866,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":16,"elapsed_ms":11.019,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":17,"elapsed_ms":10.979,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":18,"elapsed_ms":10.708,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":19,"elapsed_ms":10.734,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":10.846,"p95_ms":13.004,"max_ms":19.612}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":16.546,"tree_ms":1.349,"trajectory_ms":2.094,"fork_trajectory_ms":1.781,"total_ms":21.770,"update_ms":0.070,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":1,"read_ms":12.256,"tree_ms":0.954,"trajectory_ms":1.566,"fork_trajectory_ms":1.408,"total_ms":16.184,"update_ms":0.061,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":2,"read_ms":10.581,"tree_ms":0.866,"trajectory_ms":1.372,"fork_trajectory_ms":1.266,"total_ms":14.084,"update_ms":0.058,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":3,"read_ms":10.295,"tree_ms":0.862,"trajectory_ms":1.392,"fork_trajectory_ms":1.311,"total_ms":13.859,"update_ms":0.055,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":4,"read_ms":10.472,"tree_ms":0.869,"trajectory_ms":1.397,"fork_trajectory_ms":1.306,"total_ms":14.043,"update_ms":0.057,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":5,"read_ms":10.396,"tree_ms":0.900,"trajectory_ms":1.457,"fork_trajectory_ms":1.325,"total_ms":14.078,"update_ms":0.057,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":6,"read_ms":10.364,"tree_ms":0.919,"trajectory_ms":1.471,"fork_trajectory_ms":1.310,"total_ms":14.064,"update_ms":0.056,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":7,"read_ms":10.452,"tree_ms":0.896,"trajectory_ms":1.462,"fork_trajectory_ms":1.334,"total_ms":14.144,"update_ms":0.054,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":8,"read_ms":10.321,"tree_ms":0.855,"trajectory_ms":1.422,"fork_trajectory_ms":1.334,"total_ms":13.932,"update_ms":0.057,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":9,"read_ms":10.371,"tree_ms":0.894,"trajectory_ms":1.449,"fork_trajectory_ms":1.328,"total_ms":14.041,"update_ms":0.056,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":10,"read_ms":10.266,"tree_ms":0.859,"trajectory_ms":1.409,"fork_trajectory_ms":1.316,"total_ms":13.850,"update_ms":0.054,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":11,"read_ms":10.312,"tree_ms":0.897,"trajectory_ms":1.454,"fork_trajectory_ms":1.270,"total_ms":13.933,"update_ms":0.059,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":12,"read_ms":10.306,"tree_ms":0.904,"trajectory_ms":1.454,"fork_trajectory_ms":1.330,"total_ms":13.995,"update_ms":0.058,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":13,"read_ms":10.468,"tree_ms":0.875,"trajectory_ms":1.396,"fork_trajectory_ms":1.295,"total_ms":14.034,"update_ms":0.054,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":14,"read_ms":10.396,"tree_ms":0.859,"trajectory_ms":1.410,"fork_trajectory_ms":1.307,"total_ms":13.972,"update_ms":0.059,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":15,"read_ms":10.294,"tree_ms":0.894,"trajectory_ms":1.439,"fork_trajectory_ms":1.308,"total_ms":13.935,"update_ms":0.054,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":16,"read_ms":10.332,"tree_ms":0.871,"trajectory_ms":1.399,"fork_trajectory_ms":1.306,"total_ms":13.908,"update_ms":0.057,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":17,"read_ms":10.327,"tree_ms":0.896,"trajectory_ms":1.468,"fork_trajectory_ms":1.336,"total_ms":14.027,"update_ms":0.054,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":18,"read_ms":10.288,"tree_ms":0.900,"trajectory_ms":1.446,"fork_trajectory_ms":1.311,"total_ms":13.946,"update_ms":0.058,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":19,"read_ms":10.334,"tree_ms":0.893,"trajectory_ms":1.449,"fork_trajectory_ms":1.316,"total_ms":13.992,"update_ms":0.056,"nodes":7560,"records":4000,"fork_records":3592}
{"summary":"trajectory","file_bytes":1827596,"samples":20,"p50_ms":13.995,"p95_ms":16.184,"max_ms":21.770}
```

Long Trajectory, Dart:

```json
{"nodes":7630,"records":4000,"fork_records":3592}
{"operation":"sessionGraph","samples":20,"p50_ms":0.903,"p95_ms":2.677,"max_ms":4.269}
{"operation":"turnGroups","samples":20,"p50_ms":0.204,"p95_ms":0.688,"max_ms":0.791}
{"operation":"segments","samples":20,"p50_ms":0.044,"p95_ms":0.184,"max_ms":0.246}
{"operation":"inRange","samples":20,"p50_ms":0.031,"p95_ms":0.072,"max_ms":0.099}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.069,"p95_ms":0.254,"max_ms":1.023}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.43,"p95_ms":0.532,"max_ms":0.572}
```

## Token streaming after stderr cleanup

Observed on 2026-11-03 at `228ede81`, in the environment above, using the same release command and 20-sample workload. The stream path is now passed as a shell argument, and session exit cancels and joins a stderr reader whose drain deadline expires. Every sample preserved all 20,000 deltas as one 320,000-byte text event, six events in all. Maximum resident set size was 14,368,768 bytes; peak memory footprint was 9,372,152 bytes. This workload does not exercise a descendant holding stderr open; the session regression test covers that cleanup path.

```json
{"sample":0,"elapsed_ms":24.977,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":1,"elapsed_ms":16.965,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":2,"elapsed_ms":13.435,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":3,"elapsed_ms":11.442,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":4,"elapsed_ms":11.440,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":5,"elapsed_ms":11.296,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":6,"elapsed_ms":11.256,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":7,"elapsed_ms":11.836,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":8,"elapsed_ms":11.477,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":9,"elapsed_ms":11.650,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":10,"elapsed_ms":11.420,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":11,"elapsed_ms":11.574,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":12,"elapsed_ms":11.633,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":13,"elapsed_ms":11.629,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":14,"elapsed_ms":11.533,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":15,"elapsed_ms":11.664,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":16,"elapsed_ms":11.928,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":17,"elapsed_ms":11.498,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":18,"elapsed_ms":11.381,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":19,"elapsed_ms":11.647,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":11.574,"p95_ms":16.965,"max_ms":24.977}
```
