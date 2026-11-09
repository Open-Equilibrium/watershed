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

### Latest, at `fae2ee11`

Observed on 2026-11-09 in the revert-confirmation working tree over `fae2ee11` (ADR-0256), where a revert plan also resolves the working folder's tree and a revert also rereads that tree, the branch and what Git ignores in the way, to bind it to the confirmed plan. Another agent session was open on the host; no other builds or tests ran during measurement.

Planning a revert takes 94.0 ms p50 (114.2 ms p95) and reverting 184.3 ms p50 (215.2 ms p95). Status and stash reads take 74.4 ms p50 (81.2 ms p95), uncommitted reads 69.7 ms p50 (79.9 ms p95), and commit plans 138.1 ms p50 (147.5 ms p95).

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":134.5,"end_ms":77.962,"files":11,"list_ms":26.001,"plan_ms":106.093,"poll_ms":81.208,"revert_ms":204.463,"sample":0,"start_ms":95.27,"uncommitted_ms":78.218}
{"attributed_turns":2,"commit_plan_ms":147.543,"end_ms":80.317,"files":11,"list_ms":26.401,"plan_ms":107.146,"poll_ms":78.622,"revert_ms":215.192,"sample":1,"start_ms":71.11,"uncommitted_ms":79.918}
{"attributed_turns":3,"commit_plan_ms":136.922,"end_ms":69.942,"files":11,"list_ms":25.457,"plan_ms":102.514,"poll_ms":78.427,"revert_ms":240.221,"sample":2,"start_ms":75.662,"uncommitted_ms":72.01}
{"attributed_turns":4,"commit_plan_ms":144.812,"end_ms":101.065,"files":11,"list_ms":25.397,"plan_ms":114.814,"poll_ms":92.232,"revert_ms":186.931,"sample":3,"start_ms":79.321,"uncommitted_ms":82.56}
{"attributed_turns":5,"commit_plan_ms":127.479,"end_ms":66.907,"files":11,"list_ms":23.202,"plan_ms":89.436,"poll_ms":72.765,"revert_ms":178.064,"sample":4,"start_ms":62.908,"uncommitted_ms":65.62}
{"attributed_turns":6,"commit_plan_ms":131.129,"end_ms":67.397,"files":11,"list_ms":22.556,"plan_ms":89.917,"poll_ms":73.259,"revert_ms":179.241,"sample":5,"start_ms":62.75,"uncommitted_ms":66.868}
{"attributed_turns":7,"commit_plan_ms":129.79,"end_ms":66.659,"files":11,"list_ms":22.491,"plan_ms":90.681,"poll_ms":73.357,"revert_ms":178.824,"sample":6,"start_ms":62.735,"uncommitted_ms":66.943}
{"attributed_turns":8,"commit_plan_ms":132.865,"end_ms":67.714,"files":11,"list_ms":22.623,"plan_ms":93.467,"poll_ms":74.033,"revert_ms":179.17,"sample":7,"start_ms":62.916,"uncommitted_ms":66.874}
{"attributed_turns":9,"commit_plan_ms":133.405,"end_ms":67.673,"files":11,"list_ms":22.567,"plan_ms":91.663,"poll_ms":73.152,"revert_ms":175.952,"sample":8,"start_ms":63.339,"uncommitted_ms":67.652}
{"attributed_turns":10,"commit_plan_ms":134.073,"end_ms":68.148,"files":11,"list_ms":22.56,"plan_ms":92.021,"poll_ms":73.888,"revert_ms":183.282,"sample":9,"start_ms":63.4,"uncommitted_ms":67.948}
{"attributed_turns":11,"commit_plan_ms":135.39,"end_ms":68.295,"files":11,"list_ms":22.425,"plan_ms":92.059,"poll_ms":73.434,"revert_ms":181.515,"sample":10,"start_ms":63.532,"uncommitted_ms":68.198}
{"attributed_turns":12,"commit_plan_ms":137.619,"end_ms":69.519,"files":11,"list_ms":22.444,"plan_ms":91.869,"poll_ms":73.711,"revert_ms":183.844,"sample":11,"start_ms":64.663,"uncommitted_ms":68.614}
{"attributed_turns":13,"commit_plan_ms":138.057,"end_ms":69.606,"files":11,"list_ms":22.614,"plan_ms":92.367,"poll_ms":74.53,"revert_ms":182.46,"sample":12,"start_ms":64.552,"uncommitted_ms":68.868}
{"attributed_turns":14,"commit_plan_ms":143.488,"end_ms":69.034,"files":11,"list_ms":23.544,"plan_ms":114.216,"poll_ms":81.019,"revert_ms":184.292,"sample":13,"start_ms":65.235,"uncommitted_ms":70.011}
{"attributed_turns":15,"commit_plan_ms":141.504,"end_ms":69.82,"files":11,"list_ms":22.792,"plan_ms":93.369,"poll_ms":75.908,"revert_ms":183.979,"sample":14,"start_ms":65.44,"uncommitted_ms":69.668}
{"attributed_turns":16,"commit_plan_ms":142.071,"end_ms":70.059,"files":11,"list_ms":22.839,"plan_ms":94.019,"poll_ms":74.858,"revert_ms":186.589,"sample":15,"start_ms":66.226,"uncommitted_ms":69.706}
{"attributed_turns":17,"commit_plan_ms":143.091,"end_ms":70.289,"files":11,"list_ms":23.115,"plan_ms":93.996,"poll_ms":74.428,"revert_ms":184.418,"sample":16,"start_ms":66.04,"uncommitted_ms":69.098}
{"attributed_turns":18,"commit_plan_ms":144.834,"end_ms":70.89,"files":11,"list_ms":22.78,"plan_ms":94.365,"poll_ms":74.753,"revert_ms":185.276,"sample":17,"start_ms":66.553,"uncommitted_ms":70.2}
{"attributed_turns":19,"commit_plan_ms":146.154,"end_ms":70.695,"files":11,"list_ms":22.366,"plan_ms":94.602,"poll_ms":74.318,"revert_ms":185.4,"sample":18,"start_ms":68.854,"uncommitted_ms":71.109}
{"attributed_turns":20,"commit_plan_ms":148.213,"end_ms":71.118,"files":11,"list_ms":22.518,"plan_ms":94.76,"poll_ms":74.391,"revert_ms":187.805,"sample":19,"start_ms":66.682,"uncommitted_ms":71.254}
{"commit_plan":{"max_ms":148.213,"p50_ms":138.057,"p95_ms":147.543},"end":{"max_ms":101.065,"p50_ms":69.82,"p95_ms":80.317},"files":20000,"first_start_ms":95.27,"list":{"max_ms":26.401,"p50_ms":22.78,"p95_ms":26.001},"list_sessions_ms":6.336,"plan":{"max_ms":114.814,"p50_ms":93.996,"p95_ms":114.216},"plan_with_folders_in_place_ms":895.135,"poll":{"max_ms":92.232,"p50_ms":74.428,"p95_ms":81.208},"remove_all_ms":16.747,"revert":{"max_ms":240.221,"p50_ms":184.292,"p95_ms":215.192},"samples":20,"start":{"max_ms":79.321,"p50_ms":65.235,"p95_ms":75.662},"summary":"checkpoints","uncommitted":{"max_ms":82.56,"p50_ms":69.668,"p95_ms":79.918}}
```

```text
       25.17 real         7.14 user        35.76 sys
            12681216  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             1336314  page reclaims
               29569  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                1429  messages sent
                1429  messages received
                1429  signals received
                  45  voluntary context switches
               60783  involuntary context switches
         56650873632  instructions retired
         13038485055  cycles elapsed
             7471536  peak memory footprint
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
