# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `6f9fce2`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project. A last turn replaces a file with a folder of 20,000 folders, which the plan searches for repositories | No agent; writing the project |

## Observations

A start checkpoint takes 66.8 ms p50 (79.6 ms p95), an end checkpoint 72.0 ms p50 (86.3 ms p95), listing a turn's changes 23.6 ms p50; planning a revert takes 89.5 ms p50 (117.8 ms p95), as it captures the project once more, and the revert 173.2 ms p50 (208.8 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 90.7 ms. Planning the revert of a file whose place holds a folder of 20,000 folders took 839.9 ms, as it reads each of them to find a repository; one revert reads at most 100,000 folder entries before it leaves the rest alone. Since `54dd77f` each sample also reverts its turn, so its start and end compare only with runs since then; at `54dd77f`, before the revert checked its result, a revert took 102.2 ms p50 on the same machine, and at `69b9d01`, without reverts, a start took 82.1 ms p50 and an end 84.2 ms p50. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 6.8 MB, from the folders found in place; `/usr/bin/time` reports 12.7 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":76.381,"files":11,"list_ms":28.442,"plan_ms":96.448,"revert_ms":191.616,"sample":0,"start_ms":90.687}
{"end_ms":79.735,"files":11,"list_ms":26.819,"plan_ms":101.343,"revert_ms":197.547,"sample":1,"start_ms":71.421}
{"end_ms":84.252,"files":11,"list_ms":26.883,"plan_ms":104.259,"revert_ms":208.014,"sample":2,"start_ms":75.385}
{"end_ms":86.344,"files":11,"list_ms":26.742,"plan_ms":91.579,"revert_ms":178.146,"sample":3,"start_ms":79.601}
{"end_ms":72.577,"files":11,"list_ms":25.419,"plan_ms":117.823,"revert_ms":230.815,"sample":4,"start_ms":66.935}
{"end_ms":92.974,"files":11,"list_ms":31.156,"plan_ms":120.73,"revert_ms":208.814,"sample":5,"start_ms":89.194}
{"end_ms":74.936,"files":11,"list_ms":23.639,"plan_ms":88.021,"revert_ms":165.819,"sample":6,"start_ms":70.325}
{"end_ms":68.435,"files":11,"list_ms":23.058,"plan_ms":85.86,"revert_ms":166.007,"sample":7,"start_ms":63.353}
{"end_ms":68.218,"files":11,"list_ms":23.715,"plan_ms":86.185,"revert_ms":166.956,"sample":8,"start_ms":63.539}
{"end_ms":68.551,"files":11,"list_ms":23.195,"plan_ms":89.053,"revert_ms":167.589,"sample":9,"start_ms":64.278}
{"end_ms":68.489,"files":11,"list_ms":23.948,"plan_ms":86.533,"revert_ms":168.654,"sample":10,"start_ms":63.862}
{"end_ms":69.413,"files":11,"list_ms":24.142,"plan_ms":95.076,"revert_ms":197.424,"sample":11,"start_ms":64.122}
{"end_ms":70.252,"files":11,"list_ms":23.156,"plan_ms":87.64,"revert_ms":172.976,"sample":12,"start_ms":78.678}
{"end_ms":74.676,"files":11,"list_ms":23.072,"plan_ms":88.944,"revert_ms":171.728,"sample":13,"start_ms":64.577}
{"end_ms":70.392,"files":11,"list_ms":23.008,"plan_ms":88.192,"revert_ms":170.184,"sample":14,"start_ms":64.966}
{"end_ms":72.019,"files":11,"list_ms":23.148,"plan_ms":88.676,"revert_ms":172.008,"sample":15,"start_ms":68.251}
{"end_ms":70.386,"files":11,"list_ms":23.557,"plan_ms":91.529,"revert_ms":170.736,"sample":16,"start_ms":65.554}
{"end_ms":69.966,"files":11,"list_ms":23.554,"plan_ms":89.019,"revert_ms":173.179,"sample":17,"start_ms":66.378}
{"end_ms":71.714,"files":11,"list_ms":23.154,"plan_ms":89.7,"revert_ms":174.345,"sample":18,"start_ms":66.784}
{"end_ms":72.288,"files":11,"list_ms":23.241,"plan_ms":89.472,"revert_ms":176.862,"sample":19,"start_ms":66.945}
{"end":{"max_ms":92.974,"p50_ms":72.019,"p95_ms":86.344},"files":20000,"first_start_ms":90.687,"list":{"max_ms":31.156,"p50_ms":23.639,"p95_ms":28.442},"plan":{"max_ms":120.73,"p50_ms":89.472,"p95_ms":117.823},"plan_with_folders_in_place_ms":839.863,"revert":{"max_ms":230.815,"p50_ms":173.179,"p95_ms":208.814},"samples":20,"start":{"max_ms":89.194,"p50_ms":66.784,"p95_ms":79.601},"summary":"checkpoints"}
```
