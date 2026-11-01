# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `54dd77f`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 69.0 ms p50 (83.3 ms p95), an end checkpoint 74.6 ms p50 (96.4 ms p95), listing a turn's changes 23.7 ms p50; planning a revert takes 88.1 ms p50 (121.1 ms p95) and the revert 102.2 ms p50 (133.5 ms p95), as each captures the project once more; the first start, which also opens the project, took 86.2 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50 on the same machine. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.3 MB; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":74.605,"files":11,"list_ms":25.429,"plan_ms":94.838,"revert_ms":114.697,"sample":0,"start_ms":86.177}
{"end_ms":78.077,"files":11,"list_ms":25.657,"plan_ms":98.927,"revert_ms":117.235,"sample":1,"start_ms":72.646}
{"end_ms":82.272,"files":11,"list_ms":25.825,"plan_ms":105.719,"revert_ms":122.417,"sample":2,"start_ms":75.256}
{"end_ms":99.541,"files":11,"list_ms":25.006,"plan_ms":107.467,"revert_ms":126.879,"sample":3,"start_ms":82.301}
{"end_ms":75.454,"files":11,"list_ms":24.873,"plan_ms":91.639,"revert_ms":107.388,"sample":4,"start_ms":83.278}
{"end_ms":86.125,"files":11,"list_ms":29.275,"plan_ms":121.062,"revert_ms":133.522,"sample":5,"start_ms":68.993}
{"end_ms":90.657,"files":11,"list_ms":28.793,"plan_ms":113.72,"revert_ms":114.719,"sample":6,"start_ms":93.25}
{"end_ms":76.308,"files":11,"list_ms":23.68,"plan_ms":87.917,"revert_ms":99.318,"sample":7,"start_ms":71.22}
{"end_ms":67.638,"files":11,"list_ms":22.67,"plan_ms":85.341,"revert_ms":98.899,"sample":8,"start_ms":65.279}
{"end_ms":67.879,"files":11,"list_ms":23.23,"plan_ms":87.152,"revert_ms":110.6,"sample":9,"start_ms":64.576}
{"end_ms":67.986,"files":11,"list_ms":23.082,"plan_ms":85.719,"revert_ms":101.141,"sample":10,"start_ms":65.715}
{"end_ms":68.374,"files":11,"list_ms":23.111,"plan_ms":86.126,"revert_ms":101.882,"sample":11,"start_ms":65.73}
{"end_ms":96.399,"files":11,"list_ms":26.476,"plan_ms":138.842,"revert_ms":135.666,"sample":12,"start_ms":77.571}
{"end_ms":79.034,"files":11,"list_ms":27.36,"plan_ms":90.828,"revert_ms":100.071,"sample":13,"start_ms":74.806}
{"end_ms":68.998,"files":11,"list_ms":22.851,"plan_ms":87.014,"revert_ms":100.619,"sample":14,"start_ms":66.07}
{"end_ms":70.503,"files":11,"list_ms":22.819,"plan_ms":87.581,"revert_ms":100.341,"sample":15,"start_ms":68.25}
{"end_ms":69.232,"files":11,"list_ms":22.943,"plan_ms":87.883,"revert_ms":100.477,"sample":16,"start_ms":66.797}
{"end_ms":69.792,"files":11,"list_ms":22.813,"plan_ms":88.073,"revert_ms":102.184,"sample":17,"start_ms":67.062}
{"end_ms":69.899,"files":11,"list_ms":22.813,"plan_ms":88.055,"revert_ms":100.933,"sample":18,"start_ms":67.404}
{"end_ms":71.283,"files":11,"list_ms":22.703,"plan_ms":87.753,"revert_ms":101.243,"sample":19,"start_ms":69.467}
{"end":{"max_ms":99.541,"p50_ms":74.605,"p95_ms":96.399},"files":20000,"first_start_ms":86.177,"list":{"max_ms":29.275,"p50_ms":23.68,"p95_ms":28.793},"plan":{"max_ms":138.842,"p50_ms":88.073,"p95_ms":121.062},"revert":{"max_ms":135.666,"p50_ms":102.184,"p95_ms":133.522},"samples":20,"start":{"max_ms":93.25,"p50_ms":68.993,"p95_ms":83.278},"summary":"checkpoints"}
```
