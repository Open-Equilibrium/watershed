# Liquid agent-surface performance evidence

Observational evidence for the [agent-surface slice](../../PLAN.md) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428).
- Rust 1.99.0, release builds; Flutter 3.47.9 `flutter test` (debug JIT).
- Token streaming observed on 2026-11-12 at commit `aabc5b4c`, through pi, and on 2026-11-15 at commit `21adc99a`, through the pinned T3 Code 0.1.9; one run each.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Token streaming | `META_HARNESS_T3=<pinned t3> /usr/bin/time -l target/release/examples/stream_observe` after `cargo build --release -p liquid_bridge --example stream_observe` | One response of 20,000 text chunks of 16 bytes from a scripted ACP agent, through the pinned T3 Code server, the core and the bridge's event forwarding, which gathers streaming text for 50 ms and merges it as it arrives, to a consumer that takes and confirms each event at once, as Liquid does; 20 samples, each a fresh thread | No model; the Dart isolate and flutter_rust_bridge's transfer to Dart |

## Observations

Token streaming through T3 (`21adc99a`): every sample preserved the 20,000 chunks as one 320,000-byte text event, ten events in all, with p50 323.938 ms, p95 421.158 ms and max 495.686 ms; peak memory footprint 20.6 MB. The maximum resident set size, 1.07 GB, includes the T3 server the workload starts and stops. T3 joined the path, so the workload changed: the pi record at `aabc5b4c` (p50 13.672 ms, six events, peak memory footprint 8.3 MB) is kept for comparison, not as like-for-like.

### Raw output

Token streaming at `21adc99a`, through T3:

```json
{"sample":0,"elapsed_ms":495.686,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":1,"elapsed_ms":421.158,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":2,"elapsed_ms":340.778,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":3,"elapsed_ms":328.067,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":4,"elapsed_ms":329.701,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":5,"elapsed_ms":343.877,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":6,"elapsed_ms":338.641,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":7,"elapsed_ms":317.183,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":8,"elapsed_ms":346.640,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":9,"elapsed_ms":327.665,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":10,"elapsed_ms":319.149,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":11,"elapsed_ms":321.669,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":12,"elapsed_ms":319.450,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":13,"elapsed_ms":321.941,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":14,"elapsed_ms":349.023,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":15,"elapsed_ms":320.355,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":16,"elapsed_ms":323.938,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":17,"elapsed_ms":323.814,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":18,"elapsed_ms":314.216,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"sample":19,"elapsed_ms":319.086,"events":10,"delta_events":1,"largest_delta_bytes":320000}
{"summary":"stream","chunks_in":20000,"chunk_bytes":16,"samples":20,"p50_ms":323.938,"p95_ms":421.158,"max_ms":495.686}
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
