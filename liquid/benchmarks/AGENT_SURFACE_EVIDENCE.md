# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT).
- Token streaming observed on 2026-11-12 at commit `aabc5b4c`, through pi, and on 2026-11-16 at commit `cdb7ab01`, through the pinned T3 Code 0.1.9; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `META_HARNESS_T3=<pinned t3> /usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text chunks of 16 bytes from a scripted ACP agent, through the pinned T3 Code server, the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes and confirms each event at once, as Liquid does; 20 samples, each a fresh thread | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |

## Observations

Token streaming through T3 (`cdb7ab01`): every sample preserved the 20,000 chunks as one 320,000-byte text event, eleven events in all, one of them the turn change a new prompt now announces, with p50 328.981 ms, p95 393.972 ms and max 433.130 ms; peak memory footprint 7.0 MB. The maximum resident set size, 1.05 GB, includes the T3 server the workload starts and stops. T3 joined the path, so the workload changed: the pi record at `aabc5b4c` (p50 13.672 ms, six events, peak memory footprint 8.3 MB) is kept for comparison, not as like-for-like.

### Raw output

Token streaming at `cdb7ab01`, through T3:

```json
{"sample":0,"elapsed_ms":433.130,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":1,"elapsed_ms":393.972,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":2,"elapsed_ms":371.166,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":3,"elapsed_ms":326.582,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":4,"elapsed_ms":335.498,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":5,"elapsed_ms":350.020,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":6,"elapsed_ms":334.566,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":7,"elapsed_ms":323.555,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":8,"elapsed_ms":328.981,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":9,"elapsed_ms":324.443,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":10,"elapsed_ms":319.061,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":11,"elapsed_ms":326.139,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":12,"elapsed_ms":325.167,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":13,"elapsed_ms":322.019,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":14,"elapsed_ms":355.704,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":15,"elapsed_ms":330.637,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":16,"elapsed_ms":318.534,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":17,"elapsed_ms":337.948,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":18,"elapsed_ms":323.490,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"sample":19,"elapsed_ms":343.864,"events":11,"delta_events":1,"largest_delta_bytes":320000}
{"summary":"stream","chunks_in":20000,"chunk_bytes":16,"samples":20,"p50_ms":328.981,"p95_ms":393.972,"max_ms":433.130}
```

Token streaming at `aabc5b4c`, through pi:

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
