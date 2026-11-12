# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT).
- Token streaming observed on 2026-11-03 at commit `228ede81`, before the consumer confirmed events, and on 2026-11-12 at commit `aabc5b4c`, with the window; both Trajectory workloads on 2026-11-11 at `fbda1622`; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes and confirms each event at once, as Liquid does; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of seven entries (prompt, response with a tool call, tool result, final response and three timing entries) and eight forks of ten turns: 7,560 entries in 1,827,596 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples. A second run of the example writes the file first, so building it stays out of the measured process | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 7,630 tree nodes and 4,000 records on the main branch; the late fork shares 3,552 steps, the early fork 444; three warmups and 20 samples per operation | Widget building and painting |

## Observations

Token streaming: every sample preserved all 20,000 deltas as one 320,000-byte text event, six events in all. Maximum resident set size 14.4 MB, peak memory footprint 9.4 MB. Session exit cancels and joins a stderr reader whose drain deadline expires; this workload does not exercise a descendant holding stderr open, which the session regression test covers. The record at `7781c3f`, whose forwarder also drained the channel after the window, counted 2 text events; earlier records before the 50 ms window counted 173 to 184 events; neither is comparable. With the window (`aabc5b4c`, 2026-11-12), whose consumer confirms each event, every sample again preserved the 20,000 deltas as one 320,000-byte text event, six events in all, with p50 13.672 ms, p95 14.452 ms and max 17.288 ms; maximum resident set size 13.4 MB, peak memory footprint 8.3 MB. The consumer confirms now, so the workload changed: the `228ede81` record is kept for comparison, not as like-for-like.

Long Trajectory, core: the first read from the session file takes most of each sample (11.1 ms p50); the tree and both Trajectories over the compact index take about 4.1 ms together, and the follow-up read of new entries about 0.16 ms, which now includes asking the agent for its session id before and after the read. The process's peak memory footprint is 16.2 MB. `/usr/bin/time` reports 45.0 MB maximum resident set size, but that counts the waited-for writer run too, so only the footprint describes the measured process. Records before `21d9382c` stored an approval entry per turn and are not comparable.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 3,556 against 40 steps.

### Raw output

Token streaming at `228ede81`, before the consumer confirmed events:

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

Token streaming at `aabc5b4c`, with the window:

```json
{"sample":0,"elapsed_ms":17.288,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":1,"elapsed_ms":14.452,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":2,"elapsed_ms":13.606,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":3,"elapsed_ms":13.672,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":4,"elapsed_ms":13.455,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":5,"elapsed_ms":13.656,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":6,"elapsed_ms":13.286,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":7,"elapsed_ms":13.897,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":8,"elapsed_ms":13.593,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":9,"elapsed_ms":13.761,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":10,"elapsed_ms":13.592,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":11,"elapsed_ms":13.591,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":12,"elapsed_ms":13.740,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":13,"elapsed_ms":13.860,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":14,"elapsed_ms":13.502,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":15,"elapsed_ms":13.800,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":16,"elapsed_ms":13.587,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":17,"elapsed_ms":13.933,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":18,"elapsed_ms":13.722,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"sample":19,"elapsed_ms":14.206,"events":6,"delta_events":1,"largest_delta_bytes":320000}
{"summary":"stream","deltas_in":20000,"delta_bytes":16,"samples":20,"p50_ms":13.672,"p95_ms":14.452,"max_ms":17.288}
```

Long Trajectory, core:

```json
{"sample":0,"read_ms":11.415,"tree_ms":1.047,"trajectory_ms":1.821,"fork_trajectory_ms":1.646,"total_ms":15.928,"update_ms":0.170,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":1,"read_ms":11.167,"tree_ms":0.936,"trajectory_ms":1.692,"fork_trajectory_ms":1.516,"total_ms":15.311,"update_ms":0.158,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":2,"read_ms":11.047,"tree_ms":0.944,"trajectory_ms":1.669,"fork_trajectory_ms":1.520,"total_ms":15.180,"update_ms":0.160,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":3,"read_ms":11.159,"tree_ms":0.928,"trajectory_ms":1.677,"fork_trajectory_ms":1.537,"total_ms":15.300,"update_ms":0.158,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":4,"read_ms":11.100,"tree_ms":0.952,"trajectory_ms":1.660,"fork_trajectory_ms":1.516,"total_ms":15.227,"update_ms":0.159,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":5,"read_ms":11.070,"tree_ms":0.923,"trajectory_ms":1.650,"fork_trajectory_ms":1.513,"total_ms":15.156,"update_ms":0.153,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":6,"read_ms":11.137,"tree_ms":0.944,"trajectory_ms":1.669,"fork_trajectory_ms":1.555,"total_ms":15.305,"update_ms":0.154,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":7,"read_ms":11.047,"tree_ms":0.943,"trajectory_ms":1.703,"fork_trajectory_ms":1.518,"total_ms":15.211,"update_ms":0.159,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":8,"read_ms":11.067,"tree_ms":0.927,"trajectory_ms":1.684,"fork_trajectory_ms":1.515,"total_ms":15.193,"update_ms":0.164,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":9,"read_ms":11.057,"tree_ms":0.946,"trajectory_ms":1.706,"fork_trajectory_ms":1.529,"total_ms":15.238,"update_ms":0.152,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":10,"read_ms":11.106,"tree_ms":0.934,"trajectory_ms":1.651,"fork_trajectory_ms":1.518,"total_ms":15.209,"update_ms":0.147,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":11,"read_ms":11.064,"tree_ms":0.932,"trajectory_ms":1.654,"fork_trajectory_ms":1.526,"total_ms":15.176,"update_ms":0.156,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":12,"read_ms":11.038,"tree_ms":0.938,"trajectory_ms":1.665,"fork_trajectory_ms":1.523,"total_ms":15.165,"update_ms":0.156,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":13,"read_ms":11.133,"tree_ms":0.941,"trajectory_ms":1.661,"fork_trajectory_ms":1.510,"total_ms":15.246,"update_ms":0.159,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":14,"read_ms":11.150,"tree_ms":0.937,"trajectory_ms":1.681,"fork_trajectory_ms":1.507,"total_ms":15.275,"update_ms":0.161,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":15,"read_ms":11.079,"tree_ms":0.935,"trajectory_ms":1.706,"fork_trajectory_ms":1.514,"total_ms":15.234,"update_ms":0.163,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":16,"read_ms":11.181,"tree_ms":0.936,"trajectory_ms":1.669,"fork_trajectory_ms":1.880,"total_ms":15.666,"update_ms":0.355,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":17,"read_ms":11.330,"tree_ms":0.968,"trajectory_ms":1.712,"fork_trajectory_ms":1.499,"total_ms":15.509,"update_ms":0.164,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":18,"read_ms":11.308,"tree_ms":0.948,"trajectory_ms":1.685,"fork_trajectory_ms":1.525,"total_ms":15.466,"update_ms":0.168,"nodes":7560,"records":4000,"fork_records":3592}
{"sample":19,"read_ms":11.143,"tree_ms":0.956,"trajectory_ms":1.672,"fork_trajectory_ms":1.516,"total_ms":15.288,"update_ms":0.174,"nodes":7560,"records":4000,"fork_records":3592}
{"summary":"trajectory","file_bytes":1827596,"samples":20,"p50_ms":15.238,"p95_ms":15.666,"max_ms":15.928}
```

Long Trajectory, Dart:

```json
{"nodes":7630,"records":4000,"fork_records":3592}
{"operation":"sessionGraph","samples":20,"p50_ms":0.884,"p95_ms":2.536,"max_ms":2.618}
{"operation":"turnGroups","samples":20,"p50_ms":0.184,"p95_ms":0.692,"max_ms":0.91}
{"operation":"segments","samples":20,"p50_ms":0.04,"p95_ms":0.218,"max_ms":0.251}
{"operation":"inRange","samples":20,"p50_ms":0.031,"p95_ms":0.105,"max_ms":0.139}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.094,"p95_ms":0.2,"max_ms":0.206}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.454,"p95_ms":0.523,"max_ms":0.609}
```
