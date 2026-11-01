# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `21a6cbd`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint and the list of what it changed. The first start also copies the project's index | No agent; writing the project |

## Observations

A start checkpoint takes 80.5 ms p50 (100.1 ms p95), an end checkpoint 82.4 ms p50 (102.7 ms p95), listing a turn's changes 33.0 ms p50; the first start, which also copies the project's index, took 84.2 ms. Each checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.3 MB; `/usr/bin/time` reports 10.5 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":77.989,"files":11,"list_ms":31.099,"sample":0,"start_ms":84.224}
{"end_ms":96.607,"files":11,"list_ms":33.365,"sample":1,"start_ms":89.801}
{"end_ms":80.805,"files":11,"list_ms":31.074,"sample":2,"start_ms":62.468}
{"end_ms":79.265,"files":11,"list_ms":30.109,"sample":3,"start_ms":74.287}
{"end_ms":91.946,"files":11,"list_ms":30.545,"sample":4,"start_ms":80.572}
{"end_ms":88.554,"files":11,"list_ms":34.878,"sample":5,"start_ms":80.476}
{"end_ms":101.702,"files":11,"list_ms":32.335,"sample":6,"start_ms":111.43}
{"end_ms":80.591,"files":11,"list_ms":31.493,"sample":7,"start_ms":64.403}
{"end_ms":78.521,"files":11,"list_ms":29.971,"sample":8,"start_ms":72.283}
{"end_ms":96.011,"files":11,"list_ms":34.036,"sample":9,"start_ms":80.852}
{"end_ms":77.251,"files":11,"list_ms":33.862,"sample":10,"start_ms":73.812}
{"end_ms":81.71,"files":11,"list_ms":33.007,"sample":11,"start_ms":97.946}
{"end_ms":94.043,"files":11,"list_ms":37.972,"sample":12,"start_ms":61.819}
{"end_ms":82.407,"files":11,"list_ms":36.912,"sample":13,"start_ms":90.555}
{"end_ms":104.558,"files":11,"list_ms":42.011,"sample":14,"start_ms":100.092}
{"end_ms":92.617,"files":11,"list_ms":33.591,"sample":15,"start_ms":91.368}
{"end_ms":78.051,"files":11,"list_ms":36.232,"sample":16,"start_ms":84.417}
{"end_ms":102.743,"files":11,"list_ms":31.873,"sample":17,"start_ms":78.193}
{"end_ms":70.641,"files":11,"list_ms":32.26,"sample":18,"start_ms":65.813}
{"end_ms":71.655,"files":11,"list_ms":28.898,"sample":19,"start_ms":59.885}
{"end":{"max_ms":104.558,"p50_ms":82.407,"p95_ms":102.743},"files":20000,"first_start_ms":84.224,"list":{"max_ms":42.011,"p50_ms":33.007,"p95_ms":37.972},"samples":20,"start":{"max_ms":111.43,"p50_ms":80.476,"p95_ms":100.092},"summary":"checkpoints"}
```
