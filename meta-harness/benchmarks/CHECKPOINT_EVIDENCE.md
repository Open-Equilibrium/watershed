# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `260f92b`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 66.6 ms p50 (86.8 ms p95), an end checkpoint 71.6 ms p50 (89.9 ms p95), listing a turn's changes 24.1 ms p50; planning a revert takes 90.6 ms p50 (125.6 ms p95), as it captures the project once more, and the revert 177.3 ms p50 (229.5 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 87.4 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.4 MB; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":74.944,"files":11,"list_ms":26.202,"plan_ms":95.238,"revert_ms":187.324,"sample":0,"start_ms":87.383}
{"end_ms":78.607,"files":11,"list_ms":26.58,"plan_ms":100.462,"revert_ms":208.457,"sample":1,"start_ms":70.756}
{"end_ms":86.259,"files":11,"list_ms":26.851,"plan_ms":104.794,"revert_ms":216.063,"sample":2,"start_ms":77.522}
{"end_ms":87.931,"files":11,"list_ms":26.325,"plan_ms":94.444,"revert_ms":177.346,"sample":3,"start_ms":83.019}
{"end_ms":71.588,"files":11,"list_ms":25.433,"plan_ms":113.41,"revert_ms":229.474,"sample":4,"start_ms":66.751}
{"end_ms":89.872,"files":11,"list_ms":34.322,"plan_ms":125.616,"revert_ms":236.632,"sample":5,"start_ms":86.793}
{"end_ms":88.377,"files":11,"list_ms":27.924,"plan_ms":107.163,"revert_ms":218.399,"sample":6,"start_ms":81.241}
{"end_ms":86.135,"files":11,"list_ms":36.912,"plan_ms":110.435,"revert_ms":201.019,"sample":7,"start_ms":77.537}
{"end_ms":130.683,"files":11,"list_ms":42.333,"plan_ms":133.275,"revert_ms":220.432,"sample":8,"start_ms":113.084}
{"end_ms":77.075,"files":11,"list_ms":23.553,"plan_ms":90.307,"revert_ms":177.204,"sample":9,"start_ms":75.907}
{"end_ms":70.128,"files":11,"list_ms":24.123,"plan_ms":88.546,"revert_ms":172.0,"sample":10,"start_ms":64.76}
{"end_ms":69.275,"files":11,"list_ms":24.139,"plan_ms":88.389,"revert_ms":170.944,"sample":11,"start_ms":65.064}
{"end_ms":69.151,"files":11,"list_ms":23.039,"plan_ms":87.388,"revert_ms":170.247,"sample":12,"start_ms":64.147}
{"end_ms":69.037,"files":11,"list_ms":24.026,"plan_ms":86.897,"revert_ms":168.429,"sample":13,"start_ms":65.184}
{"end_ms":69.711,"files":11,"list_ms":23.671,"plan_ms":87.542,"revert_ms":171.367,"sample":14,"start_ms":64.762}
{"end_ms":69.814,"files":11,"list_ms":22.784,"plan_ms":86.946,"revert_ms":170.378,"sample":15,"start_ms":64.808}
{"end_ms":69.636,"files":11,"list_ms":22.966,"plan_ms":90.571,"revert_ms":171.227,"sample":16,"start_ms":64.91}
{"end_ms":70.452,"files":11,"list_ms":22.967,"plan_ms":88.429,"revert_ms":172.071,"sample":17,"start_ms":65.332}
{"end_ms":71.196,"files":11,"list_ms":23.219,"plan_ms":88.194,"revert_ms":174.207,"sample":18,"start_ms":66.594}
{"end_ms":70.925,"files":11,"list_ms":23.173,"plan_ms":89.575,"revert_ms":178.549,"sample":19,"start_ms":65.933}
{"end":{"max_ms":130.683,"p50_ms":71.588,"p95_ms":89.872},"files":20000,"first_start_ms":87.383,"list":{"max_ms":42.333,"p50_ms":24.139,"p95_ms":36.912},"plan":{"max_ms":133.275,"p50_ms":90.571,"p95_ms":125.616},"revert":{"max_ms":236.632,"p50_ms":177.346,"p95_ms":229.474},"samples":20,"start":{"max_ms":113.084,"p50_ms":66.594,"p95_ms":86.793},"summary":"checkpoints"}
```
