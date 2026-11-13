# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT).
- Token streaming observed on 2026-11-03 at commit `228ede81`, before the consumer confirmed events, and on 2026-11-12 at commit `aabc5b4c`, with the window; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `/usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text deltas of 16 bytes from a scripted agent, through the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes and confirms each event at once, as Liquid does; 20 samples, each a fresh session | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |

## Observations

Token streaming: every sample preserved all 20,000 deltas as one 320,000-byte text event, six events in all. Maximum resident set size 14.4 MB, peak memory footprint 9.4 MB. Session exit cancels and joins a stderr reader whose drain deadline expires; this workload does not exercise a descendant holding stderr open, which the session regression test covers. The record at `7781c3f`, whose forwarder also drained the channel after the window, counted 2 text events; earlier records before the 50 ms window counted 173 to 184 events; neither is comparable. With the window (`aabc5b4c`, 2026-11-12), whose consumer confirms each event, every sample again preserved the 20,000 deltas as one 320,000-byte text event, six events in all, with p50 13.672 ms, p95 14.452 ms and max 17.288 ms; maximum resident set size 13.4 MB, peak memory footprint 8.3 MB. The consumer confirms now, so the workload changed: the `228ede81` record is kept for comparison, not as like-for-like.

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
