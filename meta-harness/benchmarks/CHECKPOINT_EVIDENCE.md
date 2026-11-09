# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observation dates and candidates are identified below.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the status and stash Liquid reads after each fetch and turn, the list of what it changed, the plan of a revert to its start and that revert. From `56e75a2` each sample also reads the uncommitted scope and a commit plan over all session turns so far, from one to twenty, with eleven uncommitted files. The first start also opens the project. A last turn replaces a file with a folder of 20,000 folders, which the plan searches for repositories; then the sessions with checkpoints are listed and all checkpoints removed | No agent; writing the project; Flutter polling and rendering, bridge session-history reads, user commits, hooks and network access. Turn metadata is already loaded |

A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. A revert plan captures the project once more, and a revert captures it before and after writing the files back; one revert reads at most 100,000 folder entries. Commit attribution reads checkpoint refs once and compares all tree pairs in one bounded `diff-tree --stdin` process, with its own deadline and byte bound.

## Observations

### Latest, at `36749654`

Observed on 2026-11-03 in the transfer-fix working tree over `36749654`; no other builds or tests ran during measurement. Network serialization and cancellation are covered by the concurrent-fetch and pending-pull tests, not by this workload.

Status and stash reads take 71.5 ms p50 (78.7 ms p95); status resolves its destination once and lists configured remotes to distinguish an absent destination from several possible destinations. Uncommitted reads take 70.9 ms p50 (79.4 ms p95), and commit plans 145.8 ms p50 (167.6 ms p95).

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":124.627,"end_ms":75.675,"files":11,"list_ms":26.532,"plan_ms":95.658,"poll_ms":73.469,"revert_ms":189.136,"sample":0,"start_ms":94.604,"uncommitted_ms":76.486}
{"attributed_turns":2,"commit_plan_ms":145.813,"end_ms":78.425,"files":11,"list_ms":26.723,"plan_ms":100.218,"poll_ms":74.243,"revert_ms":197.505,"sample":1,"start_ms":70.105,"uncommitted_ms":79.362}
{"attributed_turns":3,"commit_plan_ms":134.195,"end_ms":83.354,"files":11,"list_ms":25.881,"plan_ms":95.787,"poll_ms":72.935,"revert_ms":220.473,"sample":2,"start_ms":77.685,"uncommitted_ms":70.566}
{"attributed_turns":4,"commit_plan_ms":167.607,"end_ms":94.538,"files":11,"list_ms":28.645,"plan_ms":106.134,"poll_ms":82.496,"revert_ms":180.968,"sample":3,"start_ms":90.383,"uncommitted_ms":86.072}
{"attributed_turns":5,"commit_plan_ms":154.443,"end_ms":68.872,"files":11,"list_ms":23.664,"plan_ms":88.287,"poll_ms":70.561,"revert_ms":169.472,"sample":4,"start_ms":65.695,"uncommitted_ms":69.504}
{"attributed_turns":6,"commit_plan_ms":134.941,"end_ms":68.243,"files":11,"list_ms":24.04,"plan_ms":87.093,"poll_ms":70.546,"revert_ms":168.228,"sample":5,"start_ms":63.017,"uncommitted_ms":65.02}
{"attributed_turns":7,"commit_plan_ms":131.114,"end_ms":68.204,"files":11,"list_ms":23.782,"plan_ms":87.136,"poll_ms":70.597,"revert_ms":168.386,"sample":6,"start_ms":64.714,"uncommitted_ms":67.383}
{"attributed_turns":8,"commit_plan_ms":137.483,"end_ms":70.47,"files":11,"list_ms":24.088,"plan_ms":92.176,"poll_ms":73.251,"revert_ms":169.439,"sample":7,"start_ms":64.601,"uncommitted_ms":70.867}
{"attributed_turns":9,"commit_plan_ms":134.112,"end_ms":68.939,"files":11,"list_ms":23.758,"plan_ms":87.687,"poll_ms":70.512,"revert_ms":168.457,"sample":8,"start_ms":64.703,"uncommitted_ms":68.353}
{"attributed_turns":10,"commit_plan_ms":147.847,"end_ms":69.622,"files":11,"list_ms":23.862,"plan_ms":93.049,"poll_ms":70.357,"revert_ms":174.753,"sample":9,"start_ms":64.707,"uncommitted_ms":69.049}
{"attributed_turns":11,"commit_plan_ms":138.172,"end_ms":69.908,"files":11,"list_ms":24.654,"plan_ms":89.013,"poll_ms":71.127,"revert_ms":168.481,"sample":10,"start_ms":65.743,"uncommitted_ms":69.631}
{"attributed_turns":12,"commit_plan_ms":140.84,"end_ms":69.301,"files":11,"list_ms":23.686,"plan_ms":89.671,"poll_ms":70.931,"revert_ms":178.194,"sample":11,"start_ms":65.042,"uncommitted_ms":68.962}
{"attributed_turns":13,"commit_plan_ms":139.637,"end_ms":70.317,"files":11,"list_ms":23.754,"plan_ms":88.538,"poll_ms":71.054,"revert_ms":172.171,"sample":12,"start_ms":65.172,"uncommitted_ms":69.656}
{"attributed_turns":14,"commit_plan_ms":140.464,"end_ms":70.052,"files":11,"list_ms":23.618,"plan_ms":89.595,"poll_ms":71.455,"revert_ms":192.713,"sample":13,"start_ms":65.882,"uncommitted_ms":70.057}
{"attributed_turns":15,"commit_plan_ms":175.007,"end_ms":81.507,"files":11,"list_ms":24.795,"plan_ms":102.493,"poll_ms":78.74,"revert_ms":188.318,"sample":14,"start_ms":83.331,"uncommitted_ms":72.368}
{"attributed_turns":16,"commit_plan_ms":154.658,"end_ms":74.399,"files":11,"list_ms":34.479,"plan_ms":91.081,"poll_ms":76.327,"revert_ms":177.569,"sample":15,"start_ms":71.992,"uncommitted_ms":78.977}
{"attributed_turns":17,"commit_plan_ms":147.252,"end_ms":70.832,"files":11,"list_ms":23.885,"plan_ms":91.775,"poll_ms":71.53,"revert_ms":179.245,"sample":16,"start_ms":67.212,"uncommitted_ms":73.151}
{"attributed_turns":18,"commit_plan_ms":146.197,"end_ms":71.741,"files":11,"list_ms":24.935,"plan_ms":92.287,"poll_ms":73.542,"revert_ms":175.157,"sample":17,"start_ms":69.095,"uncommitted_ms":71.926}
{"attributed_turns":19,"commit_plan_ms":147.456,"end_ms":71.368,"files":11,"list_ms":24.518,"plan_ms":91.918,"poll_ms":71.31,"revert_ms":175.978,"sample":18,"start_ms":70.892,"uncommitted_ms":74.218}
{"attributed_turns":20,"commit_plan_ms":154.439,"end_ms":71.723,"files":11,"list_ms":24.746,"plan_ms":91.534,"poll_ms":71.935,"revert_ms":180.077,"sample":19,"start_ms":66.753,"uncommitted_ms":72.419}
{"commit_plan":{"max_ms":175.007,"p50_ms":145.813,"p95_ms":167.607},"end":{"max_ms":94.538,"p50_ms":70.832,"p95_ms":83.354},"files":20000,"first_start_ms":94.604,"list":{"max_ms":34.479,"p50_ms":24.518,"p95_ms":28.645},"list_sessions_ms":7.113,"plan":{"max_ms":106.134,"p50_ms":91.775,"p95_ms":102.493},"plan_with_folders_in_place_ms":902.463,"poll":{"max_ms":82.496,"p50_ms":71.53,"p95_ms":78.74},"remove_all_ms":17.695,"revert":{"max_ms":220.473,"p50_ms":177.569,"p95_ms":197.505},"samples":20,"start":{"max_ms":90.383,"p50_ms":65.882,"p95_ms":83.331},"summary":"checkpoints","uncommitted":{"max_ms":86.072,"p50_ms":70.867,"p95_ms":79.362}}
```

```text
26.16 real         7.16 user        33.83 sys
            12681216  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             1275001  page reclaims
               28927  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                1348  messages sent
                1348  messages received
                1348  signals received
                 273  voluntary context switches
               72770  involuntary context switches
         57494652086  instructions retired
         13520773812  cycles elapsed
             7389616  peak memory footprint
```

### Before the workload change, at `a96c7d6`

Without the uncommitted scope and commit plans, status and stash reads take 44.7 ms p50 (49.7 ms p95). Status copies the index and clears its skip marks so hidden tracked edits count, serializing that private copy with checkpoint captures while leaving the project's index alone and taking no checkout lock. A start takes 65.8 ms p50 (84.7 ms p95), an end 70.8 ms p50 (88.3 ms p95), listing a turn's changes 23.5 ms p50, planning a revert 92.1 ms p50 (111.1 ms p95) and reverting 173.5 ms p50 (211.8 ms p95). The first start takes 94.3 ms, planning with folders in place 889.4 ms, listing sessions 6.8 ms and removing all checkpoints 17.3 ms. Peak memory footprint is 6.9 MB; maximum resident set size is 12.7 MB, including the writer run.

#### Raw output

```json
{"end_ms":77.157,"files":11,"list_ms":26.688,"plan_ms":99.795,"poll_ms":45.83,"revert_ms":205.586,"sample":0,"start_ms":94.347}
{"end_ms":79.141,"files":11,"list_ms":27.09,"plan_ms":104.74,"poll_ms":45.698,"revert_ms":202.824,"sample":1,"start_ms":70.417}
{"end_ms":83.721,"files":11,"list_ms":35.363,"plan_ms":123.003,"poll_ms":49.67,"revert_ms":211.767,"sample":2,"start_ms":75.325}
{"end_ms":71.864,"files":11,"list_ms":25.494,"plan_ms":96.476,"poll_ms":47.219,"revert_ms":180.654,"sample":3,"start_ms":79.556}
{"end_ms":94.234,"files":11,"list_ms":32.183,"plan_ms":111.061,"poll_ms":55.275,"revert_ms":223.54,"sample":4,"start_ms":86.304}
{"end_ms":88.349,"files":11,"list_ms":31.348,"plan_ms":94.81,"poll_ms":49.448,"revert_ms":176.056,"sample":5,"start_ms":84.742}
{"end_ms":69.26,"files":11,"list_ms":23.518,"plan_ms":88.184,"poll_ms":48.858,"revert_ms":166.72,"sample":6,"start_ms":65.358}
{"end_ms":67.93,"files":11,"list_ms":23.221,"plan_ms":89.942,"poll_ms":43.562,"revert_ms":167.695,"sample":7,"start_ms":63.342}
{"end_ms":67.657,"files":11,"list_ms":23.371,"plan_ms":86.811,"poll_ms":43.435,"revert_ms":169.807,"sample":8,"start_ms":66.785}
{"end_ms":69.159,"files":11,"list_ms":23.345,"plan_ms":90.236,"poll_ms":43.803,"revert_ms":169.421,"sample":9,"start_ms":63.643}
{"end_ms":68.081,"files":11,"list_ms":23.276,"plan_ms":90.611,"poll_ms":43.784,"revert_ms":169.461,"sample":10,"start_ms":64.308}
{"end_ms":73.087,"files":11,"list_ms":23.461,"plan_ms":90.161,"poll_ms":43.813,"revert_ms":169.854,"sample":11,"start_ms":64.099}
{"end_ms":68.407,"files":11,"list_ms":23.35,"plan_ms":90.821,"poll_ms":43.85,"revert_ms":171.23,"sample":12,"start_ms":64.389}
{"end_ms":69.318,"files":11,"list_ms":23.333,"plan_ms":91.249,"poll_ms":44.618,"revert_ms":173.543,"sample":13,"start_ms":64.6}
{"end_ms":70.781,"files":11,"list_ms":23.748,"plan_ms":92.503,"poll_ms":48.186,"revert_ms":172.585,"sample":14,"start_ms":69.415}
{"end_ms":69.533,"files":11,"list_ms":23.718,"plan_ms":91.957,"poll_ms":43.73,"revert_ms":173.702,"sample":15,"start_ms":65.457}
{"end_ms":71.206,"files":11,"list_ms":23.407,"plan_ms":92.123,"poll_ms":44.238,"revert_ms":173.248,"sample":16,"start_ms":65.798}
{"end_ms":70.604,"files":11,"list_ms":23.857,"plan_ms":96.98,"poll_ms":44.653,"revert_ms":171.647,"sample":17,"start_ms":65.484}
{"end_ms":70.764,"files":11,"list_ms":23.274,"plan_ms":91.336,"poll_ms":44.598,"revert_ms":174.992,"sample":18,"start_ms":66.057}
{"end_ms":70.73,"files":11,"list_ms":23.117,"plan_ms":92.976,"poll_ms":44.867,"revert_ms":174.607,"sample":19,"start_ms":66.555}
{"end":{"max_ms":94.234,"p50_ms":70.764,"p95_ms":88.349},"files":20000,"first_start_ms":94.347,"list":{"max_ms":35.363,"p50_ms":23.518,"p95_ms":32.183},"list_sessions_ms":6.752,"plan":{"max_ms":123.003,"p50_ms":92.123,"p95_ms":111.061},"plan_with_folders_in_place_ms":889.406,"poll":{"max_ms":55.275,"p50_ms":44.653,"p95_ms":49.67},"remove_all_ms":17.299,"revert":{"max_ms":223.54,"p50_ms":173.543,"p95_ms":211.767},"samples":20,"start":{"max_ms":86.304,"p50_ms":65.798,"p95_ms":84.742},"summary":"checkpoints"}
```
