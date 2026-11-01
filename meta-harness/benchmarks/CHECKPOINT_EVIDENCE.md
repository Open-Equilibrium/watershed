# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `ee95db2` with the workload's status step; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the status and stash Liquid reads after each fetch and turn, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project. A last turn replaces a file with a folder of 20,000 folders, which the plan searches for repositories; then the sessions with checkpoints are listed and all checkpoints removed | No agent; writing the project |

## Observations

A start checkpoint takes 67.3 ms p50 (81.5 ms p95), an end checkpoint 74.2 ms p50 (92.9 ms p95), listing a turn's changes 24.9 ms p50; planning a revert takes 95.8 ms p50 (120.9 ms p95), as it captures the project once more, and the revert 180.6 ms p50 (223.9 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 95.9 ms. Reading the branch's status and stash, which Liquid does after each background fetch every 15 seconds and after each turn, takes 37.5 ms p50 (47.1 ms p95) with 11 uncommitted files; it reads every file's state, so it grows with the project's file count, and takes no lock. Planning the revert of a file whose place holds a folder of 20,000 folders took 1219.8 ms, as it reads each of them to find a repository; one revert reads at most 100,000 folder entries before it leaves the rest alone. Listing the sessions with checkpoints, 42 refs, took 7.6 ms and removing them all in one transaction 19.7 ms. At `a6681a0`, before this workload read the status, a start took 65.2 ms p50 and a revert 173.2 ms p50 on the same machine. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 7.0 MB, from the folders found in place; `/usr/bin/time` reports 12.8 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":92.898,"files":11,"list_ms":37.312,"plan_ms":120.968,"poll_ms":47.084,"revert_ms":218.318,"sample":0,"start_ms":95.923}
{"end_ms":88.637,"files":11,"list_ms":28.605,"plan_ms":107.566,"poll_ms":39.655,"revert_ms":203.715,"sample":1,"start_ms":77.95}
{"end_ms":86.282,"files":11,"list_ms":28.68,"plan_ms":120.919,"poll_ms":37.551,"revert_ms":223.851,"sample":2,"start_ms":75.884}
{"end_ms":71.599,"files":11,"list_ms":28.475,"plan_ms":97.573,"poll_ms":37.474,"revert_ms":189.967,"sample":3,"start_ms":81.544}
{"end_ms":103.363,"files":11,"list_ms":30.439,"plan_ms":116.745,"poll_ms":44.466,"revert_ms":236.552,"sample":4,"start_ms":73.913}
{"end_ms":87.125,"files":11,"list_ms":29.214,"plan_ms":104.252,"poll_ms":45.542,"revert_ms":188.814,"sample":5,"start_ms":85.856}
{"end_ms":68.008,"files":11,"list_ms":24.77,"plan_ms":96.49,"poll_ms":35.321,"revert_ms":175.142,"sample":6,"start_ms":64.619}
{"end_ms":69.043,"files":11,"list_ms":24.018,"plan_ms":88.858,"poll_ms":34.875,"revert_ms":172.304,"sample":7,"start_ms":65.874}
{"end_ms":68.598,"files":11,"list_ms":24.433,"plan_ms":88.116,"poll_ms":34.487,"revert_ms":177.924,"sample":8,"start_ms":67.825}
{"end_ms":69.643,"files":11,"list_ms":24.759,"plan_ms":90.338,"poll_ms":35.011,"revert_ms":174.55,"sample":9,"start_ms":67.362}
{"end_ms":68.797,"files":11,"list_ms":24.147,"plan_ms":90.931,"poll_ms":34.975,"revert_ms":175.862,"sample":10,"start_ms":64.626}
{"end_ms":74.385,"files":11,"list_ms":24.367,"plan_ms":90.972,"poll_ms":36.604,"revert_ms":175.487,"sample":11,"start_ms":65.797}
{"end_ms":70.791,"files":11,"list_ms":24.981,"plan_ms":94.588,"poll_ms":35.465,"revert_ms":206.451,"sample":12,"start_ms":65.368}
{"end_ms":87.433,"files":11,"list_ms":30.516,"plan_ms":95.821,"poll_ms":41.633,"revert_ms":182.494,"sample":13,"start_ms":79.94}
{"end_ms":74.17,"files":11,"list_ms":24.328,"plan_ms":89.742,"poll_ms":41.223,"revert_ms":180.001,"sample":14,"start_ms":66.492}
{"end_ms":71.339,"files":11,"list_ms":24.593,"plan_ms":93.518,"poll_ms":36.038,"revert_ms":177.69,"sample":15,"start_ms":67.312}
{"end_ms":72.611,"files":11,"list_ms":24.399,"plan_ms":94.722,"poll_ms":36.42,"revert_ms":180.206,"sample":16,"start_ms":66.534}
{"end_ms":75.422,"files":11,"list_ms":24.677,"plan_ms":97.873,"poll_ms":37.084,"revert_ms":185.306,"sample":17,"start_ms":68.271}
{"end_ms":73.227,"files":11,"list_ms":24.862,"plan_ms":91.869,"poll_ms":37.634,"revert_ms":180.564,"sample":18,"start_ms":67.306}
{"end_ms":75.56,"files":11,"list_ms":46.711,"plan_ms":118.596,"poll_ms":70.376,"revert_ms":180.122,"sample":19,"start_ms":66.991}
{"end":{"max_ms":103.363,"p50_ms":74.17,"p95_ms":92.898},"files":20000,"first_start_ms":95.923,"list":{"max_ms":46.711,"p50_ms":24.862,"p95_ms":37.312},"list_sessions_ms":7.59,"plan":{"max_ms":120.968,"p50_ms":95.821,"p95_ms":120.919},"plan_with_folders_in_place_ms":1219.814,"poll":{"max_ms":70.376,"p50_ms":37.474,"p95_ms":47.084},"remove_all_ms":19.702,"revert":{"max_ms":236.552,"p50_ms":180.564,"p95_ms":223.851},"samples":20,"start":{"max_ms":85.856,"p50_ms":67.312,"p95_ms":81.544},"summary":"checkpoints"}
```
