# Meta-Harness checkpoint performance evidence

Observational evidence for the checkpoints of the [Liquid Git slice](../../PLAN.md#liquid-git-integration--next-slice) under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), Git 2.54.0 (Apple Git-157).
- Rust 1.99.0, release build; observed on 2026-11-01 at commit `6aa5178`; one run.

## Workload

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Checkpoints in a large project | `/usr/bin/time -l target/release/examples/checkpoint_observe` after `cargo build --release -p meta-harness-core --example checkpoint_observe` | A Git project of 20,000 committed files of 1 KiB in 200 folders, written by a second run of the example; 20 samples, each one turn: its start checkpoint, ten changed files and one new file, its end checkpoint and the list of what it changed. The first start also opens the project | No agent; writing the project |

## Observations

A start checkpoint takes 87.8 ms p50 (104.8 ms p95), an end checkpoint 87.7 ms p50 (107.0 ms p95), listing a turn's changes 25.9 ms p50; the first start, which also opens the project, took 71.2 ms. Since `6aa5178` every checkpoint starts from a fresh copy of the project's index, so a file the project ignores since an earlier checkpoint drops out; at `21a6cbd`, which reused the private index, a start took 80.5 ms p50 and an end 82.4 ms p50 on the same machine. A checkpoint rereads only the files whose index entries changed, so its time grows with the project's file count rather than its size. Peak memory footprint 2.3 MB; `/usr/bin/time` reports 10.2 MB maximum resident set size, which includes the waited-for writer run.

### Raw output

```json
{"end_ms":75.847,"files":11,"list_ms":26.256,"sample":0,"start_ms":71.231}
{"end_ms":85.505,"files":11,"list_ms":32.424,"sample":1,"start_ms":71.725}
{"end_ms":96.668,"files":11,"list_ms":32.153,"sample":2,"start_ms":89.181}
{"end_ms":87.273,"files":11,"list_ms":26.161,"sample":3,"start_ms":88.595}
{"end_ms":90.774,"files":11,"list_ms":25.797,"sample":4,"start_ms":82.449}
{"end_ms":94.305,"files":11,"list_ms":26.39,"sample":5,"start_ms":87.764}
{"end_ms":99.222,"files":11,"list_ms":26.453,"sample":6,"start_ms":90.219}
{"end_ms":103.599,"files":11,"list_ms":26.219,"sample":7,"start_ms":96.858}
{"end_ms":77.701,"files":11,"list_ms":25.605,"sample":8,"start_ms":98.413}
{"end_ms":79.001,"files":11,"list_ms":25.306,"sample":9,"start_ms":72.809}
{"end_ms":87.69,"files":11,"list_ms":25.926,"sample":10,"start_ms":92.305}
{"end_ms":106.964,"files":11,"list_ms":28.293,"sample":11,"start_ms":107.541}
{"end_ms":98.67,"files":11,"list_ms":25.522,"sample":12,"start_ms":97.341}
{"end_ms":109.458,"files":11,"list_ms":28.193,"sample":13,"start_ms":104.81}
{"end_ms":90.198,"files":11,"list_ms":23.678,"sample":14,"start_ms":86.46}
{"end_ms":75.793,"files":11,"list_ms":23.11,"sample":15,"start_ms":75.351}
{"end_ms":75.474,"files":11,"list_ms":22.7,"sample":16,"start_ms":71.021}
{"end_ms":78.487,"files":11,"list_ms":23.52,"sample":17,"start_ms":72.027}
{"end_ms":78.249,"files":11,"list_ms":22.959,"sample":18,"start_ms":71.634}
{"end_ms":80.899,"files":11,"list_ms":22.893,"sample":19,"start_ms":72.821}
{"end":{"max_ms":109.458,"p50_ms":87.69,"p95_ms":106.964},"files":20000,"first_start_ms":71.231,"list":{"max_ms":32.424,"p50_ms":25.926,"p95_ms":32.153},"samples":20,"start":{"max_ms":107.541,"p50_ms":87.764,"p95_ms":104.81},"summary":"checkpoints"}
```
