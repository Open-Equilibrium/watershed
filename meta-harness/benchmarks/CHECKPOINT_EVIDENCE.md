# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `69b9d01`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint and the list of what it changed. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 82.1 ms p50 (103.4 ms p95), an end checkpoint 84.2 ms p50 (105.0 ms p95), listing a turn's changes 25.3 ms p50; the first start, which also opens the project with two `git rev-parse` calls since `69b9d01`, took 74.6 ms. Since `6aa5178` every checkpoint starts from a fresh copy of the project's index, so a file the project ignores since an earlier checkpoint drops out; at `21a6cbd`, which reused the private index, a start took 80.5 ms p50 and an end 82.4 ms p50 on the same machine. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.3 MB; `/usr/bin/time` reports 10.2 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":74.826,"files":11,"list_ms":25.455,"sample":0,"start_ms":74.616}
{"end_ms":77.91,"files":11,"list_ms":25.114,"sample":1,"start_ms":69.93}
{"end_ms":84.192,"files":11,"list_ms":25.585,"sample":2,"start_ms":73.831}
{"end_ms":85.875,"files":11,"list_ms":25.518,"sample":3,"start_ms":77.817}
{"end_ms":89.195,"files":11,"list_ms":25.625,"sample":4,"start_ms":82.261}
{"end_ms":94.621,"files":11,"list_ms":25.803,"sample":5,"start_ms":85.171}
{"end_ms":98.315,"files":11,"list_ms":25.308,"sample":6,"start_ms":103.161}
{"end_ms":105.041,"files":11,"list_ms":25.31,"sample":7,"start_ms":93.897}
{"end_ms":75.848,"files":11,"list_ms":24.873,"sample":8,"start_ms":98.822}
{"end_ms":76.479,"files":11,"list_ms":24.532,"sample":9,"start_ms":71.806}
{"end_ms":77.616,"files":11,"list_ms":25.868,"sample":10,"start_ms":72.775}
{"end_ms":113.121,"files":11,"list_ms":29.865,"sample":11,"start_ms":107.026}
{"end_ms":101.556,"files":11,"list_ms":29.599,"sample":12,"start_ms":103.407}
{"end_ms":100.648,"files":11,"list_ms":30.195,"sample":13,"start_ms":100.278}
{"end_ms":88.842,"files":11,"list_ms":24.209,"sample":14,"start_ms":83.615}
{"end_ms":75.61,"files":11,"list_ms":22.76,"sample":15,"start_ms":82.069}
{"end_ms":73.647,"files":11,"list_ms":22.656,"sample":16,"start_ms":69.204}
{"end_ms":74.133,"files":11,"list_ms":22.639,"sample":17,"start_ms":69.72}
{"end_ms":81.984,"files":11,"list_ms":23.311,"sample":18,"start_ms":72.266}
{"end_ms":83.618,"files":11,"list_ms":23.21,"sample":19,"start_ms":77.129}
{"end":{"max_ms":113.121,"p50_ms":84.192,"p95_ms":105.041},"files":20000,"first_start_ms":74.616,"list":{"max_ms":30.195,"p50_ms":25.31,"p95_ms":29.865},"samples":20,"start":{"max_ms":107.026,"p50_ms":82.069,"p95_ms":103.407},"summary":"checkpoints"}
```
