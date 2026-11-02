# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline in memory, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428); Rust 1.99.0, release build; `automerge` 0.12.0.
- Observed on 2026-11-02 at commit `48e05e7`; three runs.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of the Page's Views in order | the same |

## Observations

| Workload | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 8.6 µs, p99 14.7 µs, max 85.2 µs | median 8.4 µs, p99 9.8 µs, max 81.2 µs | median 8.6 µs, p99 10.2 µs, max 88.7 µs |
| Adding a Text Block at the end of the long Page | median 1.17 ms, p99 2.34 ms | median 1.18 ms, p99 2.33 ms | median 1.17 ms, p99 2.32 ms |
| Reading the long Page's 1,000 Views in order | 4.46 ms | 4.47 ms | 4.48 ms |
| Peak memory footprint of the process | 15.3 MB | 15.2 MB | 15.2 MB |

A keystroke costs about what Automerge alone takes for it in the [CRDT spike](CRDT_SPIKE_EVIDENCE.md), 6–8 µs. The pipeline checks a text position against the elements beside it, not by reading the whole text; reading the text took about 40 µs per keystroke in a 5,000-character Text Block.

Adding a Text Block and reading the order both grow with the Page's Views, because the order is read from the Page document on every request. Reading each object's map in one pass instead of key by key made both about 3.5 times faster.
