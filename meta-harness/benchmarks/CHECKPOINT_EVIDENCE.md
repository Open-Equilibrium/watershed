# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `f04fa26`; one run, after a first run that another session's work on the same machine slowed by about a fifth throughout.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 65.6 ms p50 (79.2 ms p95), an end checkpoint 71.8 ms p50 (87.9 ms p95), listing a turn's changes 24.1 ms p50; planning a revert takes 89.1 ms p50 (105.5 ms p95), as it captures the project once more, and the revert 174.6 ms p50 (220.5 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 86.4 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.5 MB; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":75.478,"files":11,"list_ms":26.325,"plan_ms":95.348,"revert_ms":203.443,"sample":0,"start_ms":86.412}
{"end_ms":78.383,"files":11,"list_ms":26.293,"plan_ms":98.913,"revert_ms":198.38,"sample":1,"start_ms":70.052}
{"end_ms":84.364,"files":11,"list_ms":26.791,"plan_ms":103.987,"revert_ms":233.443,"sample":2,"start_ms":74.082}
{"end_ms":87.878,"files":11,"list_ms":26.247,"plan_ms":90.138,"revert_ms":179.365,"sample":3,"start_ms":79.151}
{"end_ms":71.804,"files":11,"list_ms":25.362,"plan_ms":116.658,"revert_ms":220.472,"sample":4,"start_ms":68.077}
{"end_ms":95.728,"files":11,"list_ms":28.313,"plan_ms":105.494,"revert_ms":203.9,"sample":5,"start_ms":84.139}
{"end_ms":75.323,"files":11,"list_ms":25.267,"plan_ms":88.561,"revert_ms":167.356,"sample":6,"start_ms":69.608}
{"end_ms":68.005,"files":11,"list_ms":23.654,"plan_ms":88.39,"revert_ms":174.627,"sample":7,"start_ms":63.646}
{"end_ms":75.739,"files":11,"list_ms":26.645,"plan_ms":88.757,"revert_ms":167.656,"sample":8,"start_ms":72.373}
{"end_ms":85.632,"files":11,"list_ms":26.709,"plan_ms":94.001,"revert_ms":169.639,"sample":9,"start_ms":63.577}
{"end_ms":68.746,"files":11,"list_ms":23.159,"plan_ms":86.447,"revert_ms":171.189,"sample":10,"start_ms":63.732}
{"end_ms":68.562,"files":11,"list_ms":23.486,"plan_ms":86.589,"revert_ms":170.225,"sample":11,"start_ms":64.128}
{"end_ms":68.61,"files":11,"list_ms":23.447,"plan_ms":88.781,"revert_ms":173.435,"sample":12,"start_ms":64.032}
{"end_ms":69.052,"files":11,"list_ms":23.159,"plan_ms":87.535,"revert_ms":170.596,"sample":13,"start_ms":64.629}
{"end_ms":70.147,"files":11,"list_ms":22.955,"plan_ms":87.927,"revert_ms":172.394,"sample":14,"start_ms":64.922}
{"end_ms":69.45,"files":11,"list_ms":23.157,"plan_ms":91.32,"revert_ms":185.473,"sample":15,"start_ms":64.781}
{"end_ms":69.719,"files":11,"list_ms":23.2,"plan_ms":88.334,"revert_ms":173.536,"sample":16,"start_ms":65.571}
{"end_ms":70.969,"files":11,"list_ms":23.159,"plan_ms":89.305,"revert_ms":175.16,"sample":17,"start_ms":65.477}
{"end_ms":71.722,"files":11,"list_ms":24.097,"plan_ms":89.143,"revert_ms":174.531,"sample":18,"start_ms":66.409}
{"end_ms":72.058,"files":11,"list_ms":23.388,"plan_ms":88.854,"revert_ms":179.059,"sample":19,"start_ms":66.231}
{"end":{"max_ms":95.728,"p50_ms":71.804,"p95_ms":87.878},"files":20000,"first_start_ms":86.412,"list":{"max_ms":28.313,"p50_ms":24.097,"p95_ms":26.791},"plan":{"max_ms":116.658,"p50_ms":89.143,"p95_ms":105.494},"revert":{"max_ms":233.443,"p50_ms":174.627,"p95_ms":220.472},"samples":20,"start":{"max_ms":84.139,"p50_ms":65.571,"p95_ms":79.151},"summary":"checkpoints"}
```
