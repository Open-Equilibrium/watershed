# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `e9d11d5`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 66.0 ms p50 (82.8 ms p95), an end checkpoint 71.0 ms p50 (87.4 ms p95), listing a turn's changes 23.3 ms p50; planning a revert takes 95.2 ms p50 (125.7 ms p95), as it captures the project once more, and the revert 176.1 ms p50 (212.6 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 87.6 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.4 MB; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":75.736,"files":11,"list_ms":25.503,"plan_ms":102.395,"revert_ms":195.949,"sample":0,"start_ms":87.599}
{"end_ms":80.884,"files":11,"list_ms":25.879,"plan_ms":107.565,"revert_ms":203.313,"sample":1,"start_ms":70.575}
{"end_ms":83.924,"files":11,"list_ms":28.06,"plan_ms":111.016,"revert_ms":212.592,"sample":2,"start_ms":76.757}
{"end_ms":87.403,"files":11,"list_ms":24.802,"plan_ms":111.218,"revert_ms":184.249,"sample":3,"start_ms":82.817}
{"end_ms":72.722,"files":11,"list_ms":25.04,"plan_ms":128.484,"revert_ms":232.046,"sample":4,"start_ms":66.737}
{"end_ms":92.621,"files":11,"list_ms":28.805,"plan_ms":125.672,"revert_ms":207.793,"sample":5,"start_ms":88.023}
{"end_ms":68.378,"files":11,"list_ms":23.332,"plan_ms":92.139,"revert_ms":171.104,"sample":6,"start_ms":65.567}
{"end_ms":68.487,"files":11,"list_ms":23.009,"plan_ms":92.812,"revert_ms":172.158,"sample":7,"start_ms":64.377}
{"end_ms":68.673,"files":11,"list_ms":22.999,"plan_ms":92.357,"revert_ms":171.905,"sample":8,"start_ms":63.931}
{"end_ms":71.824,"files":11,"list_ms":23.155,"plan_ms":93.026,"revert_ms":172.354,"sample":9,"start_ms":64.186}
{"end_ms":68.887,"files":11,"list_ms":22.946,"plan_ms":93.0,"revert_ms":172.266,"sample":10,"start_ms":64.419}
{"end_ms":69.608,"files":11,"list_ms":22.893,"plan_ms":93.183,"revert_ms":173.781,"sample":11,"start_ms":64.44}
{"end_ms":76.36,"files":11,"list_ms":25.339,"plan_ms":100.098,"revert_ms":184.72,"sample":12,"start_ms":65.97}
{"end_ms":70.078,"files":11,"list_ms":22.737,"plan_ms":95.148,"revert_ms":175.657,"sample":13,"start_ms":66.751}
{"end_ms":70.25,"files":11,"list_ms":23.733,"plan_ms":94.695,"revert_ms":175.501,"sample":14,"start_ms":65.701}
{"end_ms":71.413,"files":11,"list_ms":23.057,"plan_ms":95.173,"revert_ms":174.849,"sample":15,"start_ms":65.837}
{"end_ms":70.736,"files":11,"list_ms":22.778,"plan_ms":96.43,"revert_ms":175.826,"sample":16,"start_ms":65.778}
{"end_ms":70.812,"files":11,"list_ms":23.018,"plan_ms":95.066,"revert_ms":176.069,"sample":17,"start_ms":66.003}
{"end_ms":70.729,"files":11,"list_ms":22.64,"plan_ms":95.228,"revert_ms":177.787,"sample":18,"start_ms":66.586}
{"end_ms":70.963,"files":11,"list_ms":23.26,"plan_ms":96.168,"revert_ms":180.105,"sample":19,"start_ms":66.638}
{"end":{"max_ms":92.621,"p50_ms":70.963,"p95_ms":87.403},"files":20000,"first_start_ms":87.599,"list":{"max_ms":28.805,"p50_ms":23.26,"p95_ms":28.06},"plan":{"max_ms":128.484,"p50_ms":95.228,"p95_ms":125.672},"revert":{"max_ms":232.046,"p50_ms":176.069,"p95_ms":212.592},"samples":20,"start":{"max_ms":88.023,"p50_ms":65.97,"p95_ms":82.817},"summary":"checkpoints"}
```
