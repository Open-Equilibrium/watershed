# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT), 3.47.7 for the long approval line.
- Token streaming observed on 2026-10-31 at commit `2b34840`; the two long Trajectory workloads on 2026-10-31 at commit `9354dbc`; one run each. The long approval line observed on 2026-10-28 at `24af34f`, as one text, and with that commit's approval card replaced by pieces; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes each event at once; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |
| Long Trajectory, core | `/usr/bin/time -l target/release/examples/trajectory_observe` after `cargo build --release -p meta-harness-core --example trajectory_observe` | A session file of 1,000 turns of eight entries (prompt, response with a tool call, approval, tool result, final response and three timing entries) and eight forks of ten turns: 8,640 entries in 2,050,826 bytes; per sample a fresh session reads the stored entries from the file, asks the scripted agent for newer ones, builds the tree and the Trajectories of the main branch and the last fork, then reads again, which only asks for entries after the last one; 20 samples. A second run of the example writes the file first, so building it stays out of the measured process | No model; agent-side time to produce the data |
| Long Trajectory, Dart | `flutter test benchmark/trajectory_observe_test.dart` in `liquid/app` | The core's session shape plus a ninth, early fork of ten turns: 8,720 tree nodes and 5,000 records on the main branch; the late fork shares 4,440 steps, the early fork 555; three warmups and 20 samples per operation | Widget building and painting |
| Long approval line | `flutter test benchmark/approval_line_observe_test.dart` in `liquid/app` | A shell command of 1,000,000 characters without spaces opens as an approval in the agent shell at 1600 × 900; five samples, each from a new shell, timed until the card has settled | Rust, the bridge and the first frame of the shell |

## Observations

Token streaming: the 20,000 deltas arrive as 1 text event of 320,000 bytes, 6 events in all. Maximum resident set size 9.9 MB, peak memory footprint 8.4 MB. The record at `7781c3f`, whose forwarder also drained the channel after the window, counted 2 text events; earlier records before the 50 ms window counted 173 to 184 events; neither is comparable.

Long Trajectory, core: the first read from the session file takes most of each sample (10.8 ms p50); the tree and both Trajectories over the compact index take about 3.8 ms together, and the follow-up read of new entries about 0.06 ms. The tree takes 0.87 ms p50, up from 0.42 ms at `733887b` on 2026-10-26, a commit since rewritten, since each tool result now finds its own call through the entries above it. The process's peak memory footprint is 16.7 MB. `/usr/bin/time` reports 50.8 MB maximum resident set size, but that counts the waited-for writer run too, so only the footprint describes the measured process. Earlier records built the session file in the measured process and are not comparable, nor are runs that read the stored entries from one agent response.

Long Trajectory, Dart: every operation is linear in the steps except `compareBranches`, whose alignment is capped at 250,000 step pairs; `compareBranches.early` aligns 4,445 against 50 steps.

Long approval line: as one text, the card took 19.3 to 21.4 s to settle, since a line without spaces lays out in time that grows with the square of its length; in pieces of at most 2,000 characters it took 111 to 151 ms, most of it cutting the line once at the card's width.

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
{"sample":0,"read_ms":16.617,"tree_ms":1.234,"trajectory_ms":2.323,"fork_trajectory_ms":1.915,"total_ms":22.089,"update_ms":0.070,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":1,"read_ms":12.240,"tree_ms":0.940,"trajectory_ms":1.674,"fork_trajectory_ms":1.445,"total_ms":16.299,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":2,"read_ms":10.844,"tree_ms":0.840,"trajectory_ms":1.518,"fork_trajectory_ms":1.386,"total_ms":14.588,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":3,"read_ms":10.603,"tree_ms":0.849,"trajectory_ms":1.544,"fork_trajectory_ms":1.384,"total_ms":14.380,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":4,"read_ms":10.779,"tree_ms":0.841,"trajectory_ms":1.517,"fork_trajectory_ms":1.396,"total_ms":14.533,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":5,"read_ms":10.668,"tree_ms":0.848,"trajectory_ms":1.535,"fork_trajectory_ms":1.403,"total_ms":14.454,"update_ms":0.054,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":6,"read_ms":10.572,"tree_ms":0.838,"trajectory_ms":1.529,"fork_trajectory_ms":1.388,"total_ms":14.325,"update_ms":0.055,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":7,"read_ms":10.449,"tree_ms":0.867,"trajectory_ms":1.564,"fork_trajectory_ms":1.449,"total_ms":14.330,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":8,"read_ms":11.485,"tree_ms":1.001,"trajectory_ms":1.671,"fork_trajectory_ms":1.546,"total_ms":15.702,"update_ms":0.063,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":9,"read_ms":11.808,"tree_ms":1.044,"trajectory_ms":1.635,"fork_trajectory_ms":1.487,"total_ms":15.974,"update_ms":0.062,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":10,"read_ms":11.604,"tree_ms":0.938,"trajectory_ms":1.643,"fork_trajectory_ms":1.542,"total_ms":15.727,"update_ms":0.072,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":11,"read_ms":14.899,"tree_ms":1.597,"trajectory_ms":2.160,"fork_trajectory_ms":1.544,"total_ms":20.199,"update_ms":0.074,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":12,"read_ms":14.885,"tree_ms":1.536,"trajectory_ms":2.452,"fork_trajectory_ms":2.130,"total_ms":21.003,"update_ms":0.077,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":13,"read_ms":11.691,"tree_ms":0.937,"trajectory_ms":1.640,"fork_trajectory_ms":1.500,"total_ms":15.768,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":14,"read_ms":10.956,"tree_ms":0.884,"trajectory_ms":1.539,"fork_trajectory_ms":1.419,"total_ms":14.798,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":15,"read_ms":10.923,"tree_ms":0.839,"trajectory_ms":1.534,"fork_trajectory_ms":1.399,"total_ms":14.695,"update_ms":0.058,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":16,"read_ms":10.703,"tree_ms":0.874,"trajectory_ms":1.555,"fork_trajectory_ms":1.405,"total_ms":14.536,"update_ms":0.061,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":17,"read_ms":10.601,"tree_ms":0.859,"trajectory_ms":1.535,"fork_trajectory_ms":1.413,"total_ms":14.409,"update_ms":0.056,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":18,"read_ms":10.611,"tree_ms":0.839,"trajectory_ms":1.541,"fork_trajectory_ms":1.430,"total_ms":14.422,"update_ms":0.060,"nodes":8640,"records":5000,"fork_records":4490}
{"sample":19,"read_ms":10.691,"tree_ms":0.837,"trajectory_ms":1.533,"fork_trajectory_ms":1.407,"total_ms":14.469,"update_ms":0.059,"nodes":8640,"records":5000,"fork_records":4490}
{"summary":"trajectory","file_bytes":2050826,"samples":20,"p50_ms":14.588,"p95_ms":21.003,"max_ms":22.089}
```

Long Trajectory, Dart:

```json
{"nodes":8720,"records":5000,"fork_records":4490}
{"operation":"sessionGraph","samples":20,"p50_ms":1.31,"p95_ms":2.979,"max_ms":3.377}
{"operation":"turnGroups","samples":20,"p50_ms":0.182,"p95_ms":0.806,"max_ms":1.17}
{"operation":"segments","samples":20,"p50_ms":0.05,"p95_ms":0.082,"max_ms":0.318}
{"operation":"inRange","samples":20,"p50_ms":0.04,"p95_ms":0.09,"max_ms":0.12}
{"operation":"compareBranches.late","samples":20,"p50_ms":0.099,"p95_ms":0.336,"max_ms":0.596}
{"operation":"compareBranches.early","samples":20,"p50_ms":0.628,"p95_ms":0.759,"max_ms":0.829}
```

Long approval line, one text:

```json
{"sample":0,"opened_ms":19390.452}
{"sample":1,"opened_ms":19317.791}
{"sample":2,"opened_ms":19848.383}
{"sample":3,"opened_ms":21106.652}
{"sample":4,"opened_ms":21397.815}
```

Long approval line, pieces:

```json
{"sample":0,"opened_ms":150.972}
{"sample":1,"opened_ms":120.767}
{"sample":2,"opened_ms":112.088}
{"sample":3,"opened_ms":111.3}
{"sample":4,"opened_ms":117.474}
```
