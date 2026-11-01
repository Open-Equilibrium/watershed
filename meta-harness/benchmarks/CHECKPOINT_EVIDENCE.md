# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `a6681a0`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project. A last turn replaces a file with a folder of 20,000 folders, which the plan searches for repositories; then the sessions with checkpoints are listed and all checkpoints removed | No agent; writing the project |

## Observations

A start checkpoint takes 65.2 ms p50 (79.3 ms p95), an end checkpoint 70.6 ms p50 (87.6 ms p95), listing a turn's changes 23.3 ms p50; planning a revert takes 89.9 ms p50 (125.7 ms p95), as it captures the project once more, and the revert 173.2 ms p50 (229.4 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 87.3 ms. Planning the revert of a file whose place holds a folder of 20,000 folders took 900.9 ms, as it reads each of them to find a repository; one revert reads at most 100,000 folder entries before it leaves the rest alone. Listing the sessions with checkpoints, 42 refs, took 6.9 ms and removing them all in one transaction 17.3 ms. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 6.8 MB, from the folders found in place; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":75.189,"files":11,"list_ms":26.58,"plan_ms":95.97,"revert_ms":191.522,"sample":0,"start_ms":87.317}
{"end_ms":77.607,"files":11,"list_ms":26.745,"plan_ms":134.127,"revert_ms":207.576,"sample":1,"start_ms":69.587}
{"end_ms":87.588,"files":11,"list_ms":31.954,"plan_ms":125.662,"revert_ms":229.366,"sample":2,"start_ms":75.06}
{"end_ms":86.241,"files":11,"list_ms":25.507,"plan_ms":92.726,"revert_ms":179.138,"sample":3,"start_ms":79.272}
{"end_ms":71.759,"files":11,"list_ms":26.159,"plan_ms":121.671,"revert_ms":232.328,"sample":4,"start_ms":66.55}
{"end_ms":96.957,"files":11,"list_ms":28.492,"plan_ms":110.756,"revert_ms":215.811,"sample":5,"start_ms":76.479}
{"end_ms":75.613,"files":11,"list_ms":23.821,"plan_ms":87.847,"revert_ms":165.493,"sample":6,"start_ms":84.3}
{"end_ms":66.756,"files":11,"list_ms":22.935,"plan_ms":85.446,"revert_ms":165.488,"sample":7,"start_ms":62.29}
{"end_ms":67.519,"files":11,"list_ms":23.267,"plan_ms":88.54,"revert_ms":187.221,"sample":8,"start_ms":62.749}
{"end_ms":71.245,"files":11,"list_ms":23.094,"plan_ms":85.582,"revert_ms":163.84,"sample":9,"start_ms":63.697}
{"end_ms":67.7,"files":11,"list_ms":22.747,"plan_ms":89.502,"revert_ms":173.187,"sample":10,"start_ms":62.963}
{"end_ms":69.073,"files":11,"list_ms":23.242,"plan_ms":86.453,"revert_ms":168.291,"sample":11,"start_ms":64.114}
{"end_ms":68.809,"files":11,"list_ms":23.163,"plan_ms":86.218,"revert_ms":168.624,"sample":12,"start_ms":63.455}
{"end_ms":68.606,"files":11,"list_ms":23.683,"plan_ms":87.473,"revert_ms":169.759,"sample":13,"start_ms":64.366}
{"end_ms":70.649,"files":11,"list_ms":23.192,"plan_ms":87.733,"revert_ms":169.759,"sample":14,"start_ms":64.494}
{"end_ms":69.262,"files":11,"list_ms":23.074,"plan_ms":100.218,"revert_ms":180.327,"sample":15,"start_ms":64.45}
{"end_ms":70.294,"files":11,"list_ms":23.182,"plan_ms":91.373,"revert_ms":171.544,"sample":16,"start_ms":65.234}
{"end_ms":69.471,"files":11,"list_ms":23.191,"plan_ms":88.441,"revert_ms":172.161,"sample":17,"start_ms":65.255}
{"end_ms":70.752,"files":11,"list_ms":25.144,"plan_ms":91.655,"revert_ms":173.04,"sample":18,"start_ms":66.5}
{"end_ms":69.652,"files":11,"list_ms":23.022,"plan_ms":89.943,"revert_ms":176.515,"sample":19,"start_ms":66.782}
{"end":{"max_ms":96.957,"p50_ms":70.649,"p95_ms":87.588},"files":20000,"first_start_ms":87.317,"list":{"max_ms":31.954,"p50_ms":23.267,"p95_ms":28.492},"list_sessions_ms":6.879,"plan":{"max_ms":134.127,"p50_ms":89.943,"p95_ms":125.662},"plan_with_folders_in_place_ms":900.915,"remove_all_ms":17.273,"revert":{"max_ms":232.328,"p50_ms":173.187,"p95_ms":229.366},"samples":20,"start":{"max_ms":84.3,"p50_ms":65.234,"p95_ms":79.272},"summary":"checkpoints"}
```
