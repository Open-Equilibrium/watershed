# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observation dates and candidates are identified below.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint, the status and stash Liquid reads after each fetch and turn, the list of what it changed, the plan of a revert to its start and that revert. The first start also opens the project. A last turn replaces a file with a folder of 20,000 folders, which the plan searches for repositories; then the sessions with checkpoints are listed and all checkpoints removed | No agent; writing the project |

## Observations

### Baseline at `ee95db2`

A start checkpoint takes 67.3 ms p50 (81.5 ms p95), an end checkpoint 74.2 ms p50 (92.9 ms p95), listing a turn's changes 24.9 ms p50; planning a revert takes 95.8 ms p50 (120.9 ms p95), as it captures the project once more, and the revert 180.6 ms p50 (223.9 ms p95), as it captures the project before and after writing the files back to check them; the first start, which also opens the project, took 95.9 ms. Reading the branch's status and stash, which Liquid does after each background fetch every 15 seconds and after each turn, takes 37.5 ms p50 (47.1 ms p95) with 11 uncommitted files; it reads every file's state, so it grows with the project's file count, and takes no lock. Planning the revert of a file whose place holds a folder of 20,000 folders took 1219.8 ms, as it reads each of them to find a repository; one revert reads at most 100,000 folder entries before it leaves the rest alone. Listing the sessions with checkpoints, 42 refs, took 7.6 ms and removing them all in one transaction 19.7 ms. At `a6681a0`, before this workload read the status, a start took 65.2 ms p50 and a revert 173.2 ms p50 on the same machine. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 7.0 MB, from the folders found in place; `/usr/bin/time` reports 12.8 MB maximum resident set size, which includes the waited-for writer run.

#### Raw output

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

### Status with skip marks cleared

The same workload after `a96c7d6`, with the private-index status change, reads the branch's status and stash in 44.7 ms p50 (49.7 ms p95). Status copies the index and clears its skip marks so hidden tracked edits count, serializing that private copy with checkpoint captures while leaving the project's index alone and taking no checkout lock. A start takes 65.8 ms p50 (84.7 ms p95), an end 70.8 ms p50 (88.3 ms p95), listing a turn's changes 23.5 ms p50, planning a revert 92.1 ms p50 (111.1 ms p95) and reverting 173.5 ms p50 (211.8 ms p95). The first start takes 94.3 ms, planning with folders in place 889.4 ms, listing sessions 6.8 ms and removing all checkpoints 17.3 ms. Peak memory footprint is 6.9 MB; maximum resident set size is 12.7 MB, including the writer run. The command and inputs are unchanged.

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


### Commit attribution and uncommitted changes

Observed on 2026-11-02 in the PR B working tree over `56e75a2`, with the same environment and command, after rebuilding the release example. No other builds or test suites ran during this observation. Each sample additionally reads the uncommitted scope and a commit plan over all session turns so far, from one to twenty, with eleven uncommitted files. Reverts between samples remain part of the workload. Turn metadata is already loaded; Flutter rendering, bridge session-history reads, user commits, hooks and network access are excluded.

The uncommitted scope takes 77.8 ms p50 (97.0 ms p95, 107.5 ms maximum). Commit attribution takes 186.3 ms for one turn and 857.0 ms for twenty; the mixed one-to-twenty-turn samples have 550.9 ms p50 and 1024.9 ms p95, with 1046.8 ms maximum. Attribution captures the checkout once, then reads checkpoint trees and compares each turn and the gaps between turns; its cost grows with available turns. File attribution uses keyed lookups rather than comparing every changed file with every displayed file.

Status and stash reads take 78.4 ms p50 (101.4 ms p95), compared with 44.7 ms p50 in the preceding observation: status now also queries the merge state and remote default branch. It reuses the branch already read by status. Starts take 74.8 ms p50, ends 81.9 ms p50, revert plans 104.2 ms p50 and reverts 194.8 ms p50. The expanded workload takes 36.59 seconds wall time; peak memory footprint is 7,094,680 bytes and maximum resident set size 12,664,832 bytes, including the writer run.

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":186.277,"end_ms":75.041,"files":11,"list_ms":26.845,"plan_ms":104.234,"poll_ms":71.528,"revert_ms":194.837,"sample":0,"start_ms":94.868,"uncommitted_ms":75.509}
{"attributed_turns":2,"commit_plan_ms":246.077,"end_ms":78.887,"files":11,"list_ms":30.271,"plan_ms":103.899,"poll_ms":72.367,"revert_ms":206.679,"sample":1,"start_ms":71.146,"uncommitted_ms":77.081}
{"attributed_turns":3,"commit_plan_ms":287.238,"end_ms":75.594,"files":11,"list_ms":25.876,"plan_ms":128.348,"poll_ms":69.641,"revert_ms":267.747,"sample":2,"start_ms":66.816,"uncommitted_ms":72.163}
{"attributed_turns":4,"commit_plan_ms":438.004,"end_ms":114.307,"files":11,"list_ms":35.189,"plan_ms":122.08,"poll_ms":99.709,"revert_ms":192.489,"sample":3,"start_ms":101.496,"uncommitted_ms":107.476}
{"attributed_turns":5,"commit_plan_ms":397.845,"end_ms":81.659,"files":11,"list_ms":43.769,"plan_ms":97.413,"poll_ms":78.437,"revert_ms":238.341,"sample":4,"start_ms":80.203,"uncommitted_ms":79.57}
{"attributed_turns":6,"commit_plan_ms":494.589,"end_ms":107.338,"files":11,"list_ms":35.892,"plan_ms":113.055,"poll_ms":101.36,"revert_ms":207.43,"sample":5,"start_ms":110.834,"uncommitted_ms":84.917}
{"attributed_turns":7,"commit_plan_ms":424.158,"end_ms":95.973,"files":11,"list_ms":31.737,"plan_ms":94.378,"poll_ms":99.735,"revert_ms":183.854,"sample":6,"start_ms":76.244,"uncommitted_ms":78.062}
{"attributed_turns":8,"commit_plan_ms":429.424,"end_ms":69.297,"files":11,"list_ms":23.921,"plan_ms":87.305,"poll_ms":70.61,"revert_ms":165.668,"sample":7,"start_ms":67.532,"uncommitted_ms":67.327}
{"attributed_turns":9,"commit_plan_ms":482.631,"end_ms":81.857,"files":11,"list_ms":27.637,"plan_ms":103.101,"poll_ms":78.037,"revert_ms":175.22,"sample":8,"start_ms":63.629,"uncommitted_ms":64.785}
{"attributed_turns":10,"commit_plan_ms":503.376,"end_ms":69.497,"files":11,"list_ms":48.881,"plan_ms":85.844,"poll_ms":68.814,"revert_ms":167.596,"sample":9,"start_ms":63.427,"uncommitted_ms":78.32}
{"attributed_turns":11,"commit_plan_ms":550.908,"end_ms":68.81,"files":11,"list_ms":23.431,"plan_ms":90.185,"poll_ms":64.528,"revert_ms":183.01,"sample":10,"start_ms":62.534,"uncommitted_ms":66.487}
{"attributed_turns":12,"commit_plan_ms":749.184,"end_ms":88.521,"files":11,"list_ms":31.555,"plan_ms":129.697,"poll_ms":80.031,"revert_ms":213.723,"sample":11,"start_ms":96.806,"uncommitted_ms":77.824}
{"attributed_turns":13,"commit_plan_ms":835.531,"end_ms":97.908,"files":11,"list_ms":30.734,"plan_ms":106.894,"poll_ms":77.709,"revert_ms":188.881,"sample":12,"start_ms":77.273,"uncommitted_ms":87.524}
{"attributed_turns":14,"commit_plan_ms":880.662,"end_ms":84.215,"files":11,"list_ms":42.107,"plan_ms":104.939,"poll_ms":101.59,"revert_ms":228.527,"sample":13,"start_ms":71.899,"uncommitted_ms":90.932}
{"attributed_turns":15,"commit_plan_ms":1024.949,"end_ms":83.903,"files":11,"list_ms":35.188,"plan_ms":114.65,"poll_ms":80.098,"revert_ms":240.236,"sample":14,"start_ms":74.817,"uncommitted_ms":73.748}
{"attributed_turns":16,"commit_plan_ms":1013.724,"end_ms":83.664,"files":11,"list_ms":53.948,"plan_ms":142.539,"poll_ms":85.834,"revert_ms":216.764,"sample":15,"start_ms":79.469,"uncommitted_ms":94.527}
{"attributed_turns":17,"commit_plan_ms":1046.846,"end_ms":85.006,"files":11,"list_ms":46.568,"plan_ms":137.508,"poll_ms":80.203,"revert_ms":206.726,"sample":16,"start_ms":95.0,"uncommitted_ms":97.01}
{"attributed_turns":18,"commit_plan_ms":795.504,"end_ms":80.531,"files":11,"list_ms":34.291,"plan_ms":87.428,"poll_ms":84.649,"revert_ms":167.668,"sample":17,"start_ms":78.47,"uncommitted_ms":72.376}
{"attributed_turns":19,"commit_plan_ms":824.178,"end_ms":67.207,"files":11,"list_ms":25.904,"plan_ms":85.246,"poll_ms":65.365,"revert_ms":166.941,"sample":18,"start_ms":62.747,"uncommitted_ms":63.961}
{"attributed_turns":20,"commit_plan_ms":856.951,"end_ms":68.94,"files":11,"list_ms":23.677,"plan_ms":87.633,"poll_ms":64.581,"revert_ms":166.335,"sample":19,"start_ms":62.021,"uncommitted_ms":65.372}
{"commit_plan":{"max_ms":1046.846,"p50_ms":550.908,"p95_ms":1024.949},"end":{"max_ms":114.307,"p50_ms":81.857,"p95_ms":107.338},"files":20000,"first_start_ms":94.868,"list":{"max_ms":53.948,"p50_ms":31.737,"p95_ms":48.881},"list_sessions_ms":7.045,"plan":{"max_ms":142.539,"p50_ms":104.234,"p95_ms":137.508},"plan_with_folders_in_place_ms":929.818,"poll":{"max_ms":101.59,"p50_ms":78.437,"p95_ms":101.36},"remove_all_ms":17.65,"revert":{"max_ms":267.747,"p50_ms":194.837,"p95_ms":240.236},"samples":20,"start":{"max_ms":110.834,"p50_ms":74.817,"p95_ms":101.496},"summary":"checkpoints","uncommitted":{"max_ms":107.476,"p50_ms":77.824,"p95_ms":97.01}}
```

```text
       36.59 real        12.06 user        39.90 sys
            12664832  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             2307573  page reclaims
               39291  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                2588  messages sent
                2588  messages received
                2588  signals received
                1610  voluntary context switches
              170928  involuntary context switches
         59694009527  instructions retired
         15807969411  cycles elapsed
             7094680  peak memory footprint
```

### Batched commit attribution

Observed on 2026-11-02 in the KPI-fix working tree over `0c794c4`, in the same environment with the same expanded workload and exclusions above. Rebuilt with `cargo build --release -p meta-harness-core --example checkpoint_observe`, then ran `/usr/bin/time -l target/release/examples/checkpoint_observe`; no other builds or tests ran during measurement.

Commit plans take 147.8 ms for one turn and 128.8 ms for twenty, with 124.8 ms p50, 147.8 ms p95 and 154.1 ms maximum across the twenty samples, compared with 550.9 ms p50 and 1024.9 ms p95 above. Checkpoint refs are read once and all tree pairs use one bounded `diff-tree --stdin` process; attribution has a separate deadline and returns the file list without labels if it times out or exceeds its byte bound. Uncommitted reads take 66.3 ms p50 (81.7 ms p95); status and stash take 58.9 ms p50 (64.0 ms p95), with no default-branch query for an untracked branch. The complete workload takes 24.53 seconds wall time; peak memory footprint is 7,324,080 bytes and maximum resident set size 12,664,832 bytes, including the writer run.

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":147.787,"end_ms":83.035,"files":11,"list_ms":28.838,"plan_ms":97.45,"poll_ms":64.037,"revert_ms":198.445,"sample":0,"start_ms":108.176,"uncommitted_ms":81.722}
{"attributed_turns":2,"commit_plan_ms":136.213,"end_ms":80.66,"files":11,"list_ms":26.863,"plan_ms":96.32,"poll_ms":63.637,"revert_ms":193.735,"sample":1,"start_ms":74.74,"uncommitted_ms":76.317}
{"attributed_turns":3,"commit_plan_ms":125.426,"end_ms":80.932,"files":11,"list_ms":25.955,"plan_ms":90.699,"poll_ms":62.127,"revert_ms":201.862,"sample":2,"start_ms":71.315,"uncommitted_ms":72.121}
{"attributed_turns":4,"commit_plan_ms":154.052,"end_ms":100.554,"files":11,"list_ms":30.194,"plan_ms":109.521,"poll_ms":72.696,"revert_ms":183.575,"sample":3,"start_ms":81.173,"uncommitted_ms":83.027}
{"attributed_turns":5,"commit_plan_ms":117.356,"end_ms":69.748,"files":11,"list_ms":23.487,"plan_ms":87.719,"poll_ms":59.879,"revert_ms":166.063,"sample":4,"start_ms":67.548,"uncommitted_ms":66.366}
{"attributed_turns":6,"commit_plan_ms":121.069,"end_ms":66.996,"files":11,"list_ms":23.353,"plan_ms":83.738,"poll_ms":58.948,"revert_ms":164.552,"sample":5,"start_ms":61.949,"uncommitted_ms":64.608}
{"attributed_turns":7,"commit_plan_ms":117.761,"end_ms":65.736,"files":11,"list_ms":23.038,"plan_ms":86.078,"poll_ms":58.468,"revert_ms":165.866,"sample":6,"start_ms":61.209,"uncommitted_ms":65.715}
{"attributed_turns":8,"commit_plan_ms":118.86,"end_ms":66.29,"files":11,"list_ms":23.227,"plan_ms":86.783,"poll_ms":58.495,"revert_ms":171.051,"sample":7,"start_ms":62.184,"uncommitted_ms":64.815}
{"attributed_turns":9,"commit_plan_ms":120.899,"end_ms":73.08,"files":11,"list_ms":23.46,"plan_ms":83.282,"poll_ms":59.988,"revert_ms":164.067,"sample":8,"start_ms":68.36,"uncommitted_ms":66.57}
{"attributed_turns":10,"commit_plan_ms":119.484,"end_ms":66.637,"files":11,"list_ms":23.382,"plan_ms":85.897,"poll_ms":58.631,"revert_ms":210.402,"sample":9,"start_ms":61.161,"uncommitted_ms":66.071}
{"attributed_turns":11,"commit_plan_ms":121.834,"end_ms":66.621,"files":11,"list_ms":23.212,"plan_ms":83.406,"poll_ms":58.996,"revert_ms":165.741,"sample":10,"start_ms":62.042,"uncommitted_ms":65.982}
{"attributed_turns":12,"commit_plan_ms":121.828,"end_ms":66.341,"files":11,"list_ms":23.22,"plan_ms":85.132,"poll_ms":58.51,"revert_ms":168.991,"sample":11,"start_ms":62.186,"uncommitted_ms":66.276}
{"attributed_turns":13,"commit_plan_ms":122.355,"end_ms":66.41,"files":11,"list_ms":23.11,"plan_ms":84.576,"poll_ms":58.006,"revert_ms":174.357,"sample":12,"start_ms":61.22,"uncommitted_ms":64.415}
{"attributed_turns":14,"commit_plan_ms":124.529,"end_ms":65.726,"files":11,"list_ms":23.251,"plan_ms":84.06,"poll_ms":58.439,"revert_ms":165.742,"sample":13,"start_ms":62.147,"uncommitted_ms":66.275}
{"attributed_turns":15,"commit_plan_ms":125.936,"end_ms":66.546,"files":11,"list_ms":23.9,"plan_ms":82.934,"poll_ms":61.883,"revert_ms":165.09,"sample":14,"start_ms":61.756,"uncommitted_ms":66.374}
{"attributed_turns":16,"commit_plan_ms":124.819,"end_ms":65.73,"files":11,"list_ms":23.18,"plan_ms":84.393,"poll_ms":58.609,"revert_ms":167.564,"sample":15,"start_ms":61.441,"uncommitted_ms":65.85}
{"attributed_turns":17,"commit_plan_ms":125.559,"end_ms":66.117,"files":11,"list_ms":23.07,"plan_ms":84.996,"poll_ms":58.472,"revert_ms":164.893,"sample":16,"start_ms":62.157,"uncommitted_ms":65.84}
{"attributed_turns":18,"commit_plan_ms":126.981,"end_ms":66.19,"files":11,"list_ms":23.277,"plan_ms":84.716,"poll_ms":58.931,"revert_ms":171.558,"sample":17,"start_ms":60.717,"uncommitted_ms":67.517}
{"attributed_turns":19,"commit_plan_ms":127.191,"end_ms":66.436,"files":11,"list_ms":24.467,"plan_ms":84.593,"poll_ms":61.39,"revert_ms":165.242,"sample":18,"start_ms":65.714,"uncommitted_ms":67.076}
{"attributed_turns":20,"commit_plan_ms":128.84,"end_ms":66.782,"files":11,"list_ms":23.253,"plan_ms":85.747,"poll_ms":58.285,"revert_ms":166.283,"sample":19,"start_ms":62.012,"uncommitted_ms":62.753}
{"commit_plan":{"max_ms":154.052,"p50_ms":124.819,"p95_ms":147.787},"end":{"max_ms":100.554,"p50_ms":66.621,"p95_ms":83.035},"files":20000,"first_start_ms":108.176,"list":{"max_ms":30.194,"p50_ms":23.353,"p95_ms":28.838},"list_sessions_ms":6.84,"plan":{"max_ms":109.521,"p50_ms":85.132,"p95_ms":97.45},"plan_with_folders_in_place_ms":888.3,"poll":{"max_ms":72.696,"p50_ms":58.948,"p95_ms":64.037},"remove_all_ms":17.434,"revert":{"max_ms":210.402,"p50_ms":167.564,"p95_ms":201.862},"samples":20,"start":{"max_ms":81.173,"p50_ms":62.147,"p95_ms":74.74},"summary":"checkpoints","uncommitted":{"max_ms":83.027,"p50_ms":66.276,"p95_ms":81.722}}
```

```text
       24.53 real         6.60 user        33.72 sys
            12664832  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             1223648  page reclaims
               26506  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                1288  messages sent
                1288  messages received
                1288  signals received
                2039  voluntary context switches
               64802  involuntary context switches
         56762227327  instructions retired
         13240989986  cycles elapsed
             7324080  peak memory footprint
```


### Live uncommitted refresh

Observed on 2026-11-02 in the partial-staging fix working tree over `f0464fc`, with the same expanded workload, environment, commands and exclusions above; no other builds or tests ran during measurement. Uncommitted reads take 67.2 ms p50 (79.9 ms p95), compared with 66.3 ms p50 above; commit plans take 137.1 ms p50 (154.6 ms p95). Wall time is 24.80 seconds, maximum resident set size 12,681,216 bytes and peak memory footprint 7,438,768 bytes.

Each successful status poll now also reads uncommitted changes while that scope is selected and refreshes the commit plan while the dialog is open, even when summary counts match. Uncommitted reads return the two already captured tree identities without another Git process; equal identities preserve the displayed patch and scroll position. Partial-staging detection makes one additional private index copy and clears its skip marks before reading unstaged changes. These per-read observations exclude Flutter polling and rendering; status, uncommitted and plan work is serial within each observation sample.

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":124.75,"end_ms":74.848,"files":11,"list_ms":28.865,"plan_ms":96.309,"poll_ms":62.527,"revert_ms":189.554,"sample":0,"start_ms":93.621,"uncommitted_ms":76.225}
{"attributed_turns":2,"commit_plan_ms":146.795,"end_ms":80.297,"files":11,"list_ms":26.22,"plan_ms":100.303,"poll_ms":63.456,"revert_ms":198.654,"sample":1,"start_ms":76.639,"uncommitted_ms":79.888}
{"attributed_turns":3,"commit_plan_ms":135.82,"end_ms":83.243,"files":11,"list_ms":25.582,"plan_ms":92.742,"poll_ms":62.073,"revert_ms":192.526,"sample":2,"start_ms":75.24,"uncommitted_ms":72.642}
{"attributed_turns":4,"commit_plan_ms":171.214,"end_ms":96.074,"files":11,"list_ms":30.434,"plan_ms":113.124,"poll_ms":80.445,"revert_ms":199.848,"sample":3,"start_ms":88.903,"uncommitted_ms":97.431}
{"attributed_turns":5,"commit_plan_ms":130.418,"end_ms":67.142,"files":11,"list_ms":23.494,"plan_ms":86.557,"poll_ms":58.562,"revert_ms":167.683,"sample":4,"start_ms":64.434,"uncommitted_ms":65.012}
{"attributed_turns":6,"commit_plan_ms":133.198,"end_ms":66.747,"files":11,"list_ms":23.012,"plan_ms":88.004,"poll_ms":58.671,"revert_ms":167.018,"sample":5,"start_ms":63.726,"uncommitted_ms":65.702}
{"attributed_turns":7,"commit_plan_ms":128.884,"end_ms":66.589,"files":11,"list_ms":23.165,"plan_ms":85.064,"poll_ms":58.647,"revert_ms":164.916,"sample":6,"start_ms":63.751,"uncommitted_ms":63.706}
{"attributed_turns":8,"commit_plan_ms":131.226,"end_ms":67.065,"files":11,"list_ms":23.164,"plan_ms":89.644,"poll_ms":58.705,"revert_ms":167.579,"sample":7,"start_ms":63.683,"uncommitted_ms":63.899}
{"attributed_turns":9,"commit_plan_ms":133.985,"end_ms":68.418,"files":11,"list_ms":23.42,"plan_ms":86.213,"poll_ms":58.903,"revert_ms":166.609,"sample":8,"start_ms":63.292,"uncommitted_ms":65.225}
{"attributed_turns":10,"commit_plan_ms":133.885,"end_ms":67.861,"files":11,"list_ms":23.17,"plan_ms":86.293,"poll_ms":60.807,"revert_ms":170.42,"sample":9,"start_ms":63.841,"uncommitted_ms":65.472}
{"attributed_turns":11,"commit_plan_ms":133.979,"end_ms":72.239,"files":11,"list_ms":23.994,"plan_ms":87.424,"poll_ms":66.285,"revert_ms":167.474,"sample":10,"start_ms":63.866,"uncommitted_ms":65.277}
{"attributed_turns":12,"commit_plan_ms":135.364,"end_ms":68.933,"files":11,"list_ms":23.011,"plan_ms":90.897,"poll_ms":59.211,"revert_ms":171.122,"sample":11,"start_ms":64.101,"uncommitted_ms":64.921}
{"attributed_turns":13,"commit_plan_ms":137.088,"end_ms":69.136,"files":11,"list_ms":23.254,"plan_ms":88.861,"poll_ms":60.037,"revert_ms":169.341,"sample":12,"start_ms":65.949,"uncommitted_ms":66.699}
{"attributed_turns":14,"commit_plan_ms":138.807,"end_ms":68.788,"files":11,"list_ms":23.09,"plan_ms":88.275,"poll_ms":61.206,"revert_ms":186.017,"sample":13,"start_ms":66.516,"uncommitted_ms":67.178}
{"attributed_turns":15,"commit_plan_ms":145.138,"end_ms":70.362,"files":11,"list_ms":23.537,"plan_ms":88.608,"poll_ms":59.896,"revert_ms":170.823,"sample":14,"start_ms":71.557,"uncommitted_ms":69.236}
{"attributed_turns":16,"commit_plan_ms":146.855,"end_ms":70.252,"files":11,"list_ms":23.114,"plan_ms":89.299,"poll_ms":60.185,"revert_ms":180.832,"sample":15,"start_ms":68.086,"uncommitted_ms":68.68}
{"attributed_turns":17,"commit_plan_ms":142.992,"end_ms":69.737,"files":11,"list_ms":22.939,"plan_ms":88.882,"poll_ms":60.28,"revert_ms":171.819,"sample":16,"start_ms":68.899,"uncommitted_ms":66.59}
{"attributed_turns":18,"commit_plan_ms":146.508,"end_ms":73.099,"files":11,"list_ms":23.426,"plan_ms":89.691,"poll_ms":61.618,"revert_ms":175.647,"sample":17,"start_ms":70.97,"uncommitted_ms":67.49}
{"attributed_turns":19,"commit_plan_ms":145.379,"end_ms":70.669,"files":11,"list_ms":23.227,"plan_ms":92.645,"poll_ms":60.505,"revert_ms":175.96,"sample":18,"start_ms":67.074,"uncommitted_ms":69.378}
{"attributed_turns":20,"commit_plan_ms":154.581,"end_ms":71.012,"files":11,"list_ms":23.176,"plan_ms":91.188,"poll_ms":60.77,"revert_ms":176.156,"sample":19,"start_ms":66.681,"uncommitted_ms":70.807}
{"commit_plan":{"max_ms":171.214,"p50_ms":137.088,"p95_ms":154.581},"end":{"max_ms":96.074,"p50_ms":70.252,"p95_ms":83.243},"files":20000,"first_start_ms":93.621,"list":{"max_ms":30.434,"p50_ms":23.254,"p95_ms":28.865},"list_sessions_ms":6.841,"plan":{"max_ms":113.124,"p50_ms":89.299,"p95_ms":100.303},"plan_with_folders_in_place_ms":905.548,"poll":{"max_ms":80.445,"p50_ms":60.505,"p95_ms":66.285},"remove_all_ms":31.421,"revert":{"max_ms":199.848,"p50_ms":171.819,"p95_ms":198.654},"samples":20,"start":{"max_ms":88.903,"p50_ms":66.516,"p95_ms":76.639},"summary":"checkpoints","uncommitted":{"max_ms":97.431,"p50_ms":67.178,"p95_ms":79.888}}
```

```text
       24.80 real         6.83 user        34.38 sys
            12681216  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             1246178  page reclaims
               28646  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                1308  messages sent
                1308  messages received
                1308  signals received
                 669  voluntary context switches
               70248  involuntary context switches
         57012728733  instructions retired
         13269513442  cycles elapsed
             7438768  peak memory footprint
```

### Default branch fallback

Observed on 2026-11-02 in the default-branch fix working tree over `a96a035`, with the same expanded workload, environment, commands and exclusions above; no other builds or tests ran during measurement. Status without an upstream now queries `origin/HEAD`, even when that remote is absent. Polling takes 66.2 ms p50 (70.2 ms p95), compared with 60.5 ms p50 (66.3 ms p95) in the preceding observation. Uncommitted reads take 69.2 ms p50 (87.4 ms p95); commit plans take 141.1 ms p50 (166.9 ms p95). Wall time is 25.38 seconds, maximum resident set size 12,746,752 bytes and peak memory footprint 7,209,392 bytes.

#### Raw output

```json
{"attributed_turns":1,"commit_plan_ms":126.341,"end_ms":76.916,"files":11,"list_ms":26.69,"plan_ms":97.105,"poll_ms":68.727,"revert_ms":192.713,"sample":0,"start_ms":93.237,"uncommitted_ms":76.305}
{"attributed_turns":2,"commit_plan_ms":147.475,"end_ms":84.838,"files":11,"list_ms":26.608,"plan_ms":102.591,"poll_ms":69.956,"revert_ms":201.921,"sample":1,"start_ms":71.206,"uncommitted_ms":79.763}
{"attributed_turns":3,"commit_plan_ms":137.037,"end_ms":83.6,"files":11,"list_ms":25.533,"plan_ms":93.983,"poll_ms":69.159,"revert_ms":247.117,"sample":2,"start_ms":76.149,"uncommitted_ms":72.156}
{"attributed_turns":4,"commit_plan_ms":166.894,"end_ms":91.28,"files":11,"list_ms":35.129,"plan_ms":104.48,"poll_ms":93.075,"revert_ms":180.36,"sample":3,"start_ms":91.603,"uncommitted_ms":87.363}
{"attributed_turns":5,"commit_plan_ms":131.765,"end_ms":67.533,"files":11,"list_ms":23.153,"plan_ms":85.894,"poll_ms":64.0,"revert_ms":165.63,"sample":4,"start_ms":63.022,"uncommitted_ms":64.104}
{"attributed_turns":6,"commit_plan_ms":132.163,"end_ms":67.453,"files":11,"list_ms":22.914,"plan_ms":92.176,"poll_ms":64.195,"revert_ms":173.149,"sample":5,"start_ms":62.418,"uncommitted_ms":65.674}
{"attributed_turns":7,"commit_plan_ms":131.003,"end_ms":68.554,"files":11,"list_ms":23.149,"plan_ms":86.459,"poll_ms":65.108,"revert_ms":168.546,"sample":6,"start_ms":64.687,"uncommitted_ms":66.223}
{"attributed_turns":8,"commit_plan_ms":132.636,"end_ms":68.87,"files":11,"list_ms":23.74,"plan_ms":91.654,"poll_ms":65.01,"revert_ms":173.468,"sample":7,"start_ms":64.574,"uncommitted_ms":66.278}
{"attributed_turns":9,"commit_plan_ms":170.677,"end_ms":68.505,"files":11,"list_ms":30.75,"plan_ms":89.617,"poll_ms":66.371,"revert_ms":168.634,"sample":8,"start_ms":63.967,"uncommitted_ms":93.28}
{"attributed_turns":10,"commit_plan_ms":149.509,"end_ms":68.646,"files":11,"list_ms":23.046,"plan_ms":86.963,"poll_ms":65.045,"revert_ms":171.017,"sample":9,"start_ms":64.446,"uncommitted_ms":71.834}
{"attributed_turns":11,"commit_plan_ms":136.709,"end_ms":69.161,"files":11,"list_ms":23.092,"plan_ms":87.932,"poll_ms":65.416,"revert_ms":172.192,"sample":10,"start_ms":64.266,"uncommitted_ms":67.657}
{"attributed_turns":12,"commit_plan_ms":138.223,"end_ms":71.352,"files":11,"list_ms":23.222,"plan_ms":88.473,"poll_ms":65.544,"revert_ms":174.102,"sample":11,"start_ms":66.383,"uncommitted_ms":67.428}
{"attributed_turns":13,"commit_plan_ms":150.931,"end_ms":76.149,"files":11,"list_ms":22.828,"plan_ms":95.3,"poll_ms":70.197,"revert_ms":180.732,"sample":12,"start_ms":66.648,"uncommitted_ms":70.112}
{"attributed_turns":14,"commit_plan_ms":141.089,"end_ms":70.178,"files":11,"list_ms":23.654,"plan_ms":89.3,"poll_ms":67.558,"revert_ms":171.066,"sample":13,"start_ms":65.548,"uncommitted_ms":68.672}
{"attributed_turns":15,"commit_plan_ms":140.972,"end_ms":69.355,"files":11,"list_ms":23.131,"plan_ms":88.435,"poll_ms":64.84,"revert_ms":171.057,"sample":14,"start_ms":64.879,"uncommitted_ms":68.382}
{"attributed_turns":16,"commit_plan_ms":140.971,"end_ms":70.528,"files":11,"list_ms":22.884,"plan_ms":88.382,"poll_ms":64.988,"revert_ms":173.696,"sample":15,"start_ms":65.118,"uncommitted_ms":68.382}
{"attributed_turns":17,"commit_plan_ms":143.976,"end_ms":69.753,"files":11,"list_ms":23.014,"plan_ms":91.183,"poll_ms":66.183,"revert_ms":176.791,"sample":16,"start_ms":65.997,"uncommitted_ms":69.162}
{"attributed_turns":18,"commit_plan_ms":148.478,"end_ms":71.171,"files":11,"list_ms":23.385,"plan_ms":103.346,"poll_ms":66.307,"revert_ms":176.069,"sample":17,"start_ms":66.867,"uncommitted_ms":67.011}
{"attributed_turns":19,"commit_plan_ms":150.602,"end_ms":70.998,"files":11,"list_ms":24.33,"plan_ms":90.379,"poll_ms":66.865,"revert_ms":174.033,"sample":18,"start_ms":69.292,"uncommitted_ms":69.842}
{"attributed_turns":20,"commit_plan_ms":147.612,"end_ms":71.252,"files":11,"list_ms":22.984,"plan_ms":90.072,"poll_ms":65.902,"revert_ms":173.81,"sample":19,"start_ms":66.667,"uncommitted_ms":69.98}
{"commit_plan":{"max_ms":170.677,"p50_ms":141.089,"p95_ms":166.894},"end":{"max_ms":91.28,"p50_ms":70.528,"p95_ms":84.838},"files":20000,"first_start_ms":93.237,"list":{"max_ms":35.129,"p50_ms":23.222,"p95_ms":30.75},"list_sessions_ms":6.75,"plan":{"max_ms":104.48,"p50_ms":90.379,"p95_ms":103.346},"plan_with_folders_in_place_ms":866.119,"poll":{"max_ms":93.075,"p50_ms":66.183,"p95_ms":70.197},"remove_all_ms":16.862,"revert":{"max_ms":247.117,"p50_ms":173.81,"p95_ms":201.921},"samples":20,"start":{"max_ms":91.603,"p50_ms":65.548,"p95_ms":76.149},"summary":"checkpoints","uncommitted":{"max_ms":93.28,"p50_ms":69.162,"p95_ms":87.363}}
```

```text
       25.38 real         6.93 user        35.55 sys
            12746752  maximum resident set size
                   0  average shared memory size
                   0  average unshared data size
                   0  average unshared stack size
             1260645  page reclaims
               28785  page faults
                   0  swaps
                   0  block input operations
                   0  block output operations
                1328  messages sent
                1328  messages received
                1328  signals received
                2507  voluntary context switches
               79014  involuntary context switches
         57021194546  instructions retired
         13351546243  cycles elapsed
             7209392  peak memory footprint
```
