# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `995fd8c`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 71.3 ms p50 (101.9 ms p95), an end checkpoint 76.9 ms p50 (100.7 ms p95), listing a turn's changes 26.3 ms p50; planning a revert takes 99.3 ms p50 (139.9 ms p95), as it captures the project once more, and the revert 187.8 ms p50 (257.2 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 89.5 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.4 MB; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":75.095,"files":11,"list_ms":26.175,"plan_ms":95.289,"revert_ms":189.043,"sample":0,"start_ms":89.473}
{"end_ms":78.65,"files":11,"list_ms":27.631,"plan_ms":139.872,"revert_ms":252.683,"sample":1,"start_ms":70.607}
{"end_ms":91.531,"files":11,"list_ms":27.956,"plan_ms":104.057,"revert_ms":205.685,"sample":2,"start_ms":85.052}
{"end_ms":85.44,"files":11,"list_ms":26.105,"plan_ms":112.287,"revert_ms":179.53,"sample":3,"start_ms":78.821}
{"end_ms":72.512,"files":11,"list_ms":26.101,"plan_ms":96.448,"revert_ms":221.105,"sample":4,"start_ms":66.618}
{"end_ms":93.391,"files":11,"list_ms":27.517,"plan_ms":118.919,"revert_ms":197.657,"sample":5,"start_ms":89.719}
{"end_ms":75.051,"files":11,"list_ms":23.981,"plan_ms":87.572,"revert_ms":165.251,"sample":6,"start_ms":69.73}
{"end_ms":67.496,"files":11,"list_ms":23.334,"plan_ms":86.397,"revert_ms":166.859,"sample":7,"start_ms":62.488}
{"end_ms":67.73,"files":11,"list_ms":22.951,"plan_ms":85.391,"revert_ms":163.763,"sample":8,"start_ms":62.659}
{"end_ms":71.04,"files":11,"list_ms":23.658,"plan_ms":85.994,"revert_ms":166.853,"sample":9,"start_ms":62.694}
{"end_ms":67.628,"files":11,"list_ms":22.741,"plan_ms":85.987,"revert_ms":167.324,"sample":10,"start_ms":62.818}
{"end_ms":68.501,"files":11,"list_ms":23.113,"plan_ms":87.066,"revert_ms":167.915,"sample":11,"start_ms":63.429}
{"end_ms":68.444,"files":11,"list_ms":23.131,"plan_ms":86.706,"revert_ms":183.041,"sample":12,"start_ms":63.624}
{"end_ms":77.773,"files":11,"list_ms":27.827,"plan_ms":99.329,"revert_ms":186.011,"sample":13,"start_ms":73.65}
{"end_ms":76.892,"files":11,"list_ms":26.637,"plan_ms":99.51,"revert_ms":187.752,"sample":14,"start_ms":71.516}
{"end_ms":76.877,"files":11,"list_ms":26.273,"plan_ms":123.894,"revert_ms":257.174,"sample":15,"start_ms":71.338}
{"end_ms":111.693,"files":11,"list_ms":44.144,"plan_ms":133.118,"revert_ms":213.492,"sample":16,"start_ms":112.1}
{"end_ms":93.177,"files":11,"list_ms":36.656,"plan_ms":148.584,"revert_ms":310.553,"sample":17,"start_ms":94.865}
{"end_ms":89.335,"files":11,"list_ms":28.249,"plan_ms":107.471,"revert_ms":202.188,"sample":18,"start_ms":101.917}
{"end_ms":100.714,"files":11,"list_ms":28.219,"plan_ms":92.681,"revert_ms":178.109,"sample":19,"start_ms":81.568}
{"end":{"max_ms":111.693,"p50_ms":76.892,"p95_ms":100.714},"files":20000,"first_start_ms":89.473,"list":{"max_ms":44.144,"p50_ms":26.273,"p95_ms":36.656},"plan":{"max_ms":148.584,"p50_ms":99.329,"p95_ms":139.872},"revert":{"max_ms":310.553,"p50_ms":187.752,"p95_ms":257.174},"samples":20,"start":{"max_ms":112.1,"p50_ms":71.338,"p95_ms":101.917},"summary":"checkpoints"}
```
