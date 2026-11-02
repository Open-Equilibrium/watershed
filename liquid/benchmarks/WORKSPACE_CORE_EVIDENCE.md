# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- The pipeline observed on 2026-11-02 at commit `89700d9`, the replica at `7d7a4f1`; three runs each on an otherwise idle machine. A set of runs at the same commit while other builds ran took about 35 % longer and is not comparable.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of the Page's Views in order | the same |
| Durable typing | `/usr/bin/time -l target/release/examples/replica_observe` after `cargo build --release -p liquid-storage --example replica_observe` | 2,000 keystrokes appended to one Text Block in an encrypted replica, each its own Action and durable commit | Bridge and UI |
| Reopening a long Page | the same run | 1,000 more Text Blocks on that Page; then a new session opens the replica and reads the Page's Views in order | the same |

## Observations

| Workload | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 8.7 µs, p99 16.9 µs, max 84.7 µs | median 8.6 µs, p99 10.2 µs, max 79.7 µs | median 8.6 µs, p99 10.0 µs, max 90.3 µs |
| Adding a Text Block at the end of the long Page | median 1.19 ms, p99 2.35 ms | median 1.19 ms, p99 2.34 ms | median 1.17 ms, p99 2.35 ms |
| Reading the long Page's 1,000 Views in order | 4.56 ms | 4.54 ms | 4.65 ms |
| Peak memory footprint of the pipeline process | 14.9 MB | 14.9 MB | 15.0 MB |
| Durable typing, per keystroke | median 3.10 ms, p99 6.73 ms, max 13.0 ms | median 3.03 ms, p99 5.94 ms, max 13.1 ms | median 3.07 ms, p99 6.80 ms, max 10.0 ms |
| Reopening the long Page and reading its 1,001 Views | 15.4 ms | 14.4 ms | 16.4 ms |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |
| Peak memory footprint of the replica process | 24.7 MB | 24.6 MB | 24.2 MB |

A keystroke costs about what Automerge alone takes for it in the [CRDT spike](CRDT_SPIKE_EVIDENCE.md), 6–8 µs. The pipeline checks a text position against the elements beside it, not by reading the whole text; reading the text took about 40 µs per keystroke in a 5,000-character Text Block.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). The same run without it took 72 µs per keystroke. The editor groups keystrokes into one commit per pause (client architecture rule 9), and typing never waits for a commit (rule 7).

Adding a Text Block and reading the order both grow with the Page's Views, because the order is read from the Page document on every request. Reading each object's map in one pass instead of key by key made both about 3.5 times faster.

### Raw output

Pipeline:

```text
typing, 10000 keystrokes into one Text Block, per keystroke: median 8.7 µs, p99 16.9 µs, max 84.7 µs
Page of 1000 Text Blocks, per Block added at the end: median 1186.2 µs, p99 2346.8 µs, max 2379.2 µs
reading that Page's 1000 Views in order: 4.56 ms
            14926256  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 8.6 µs, p99 10.2 µs, max 79.7 µs
Page of 1000 Text Blocks, per Block added at the end: median 1189.0 µs, p99 2335.6 µs, max 2415.2 µs
reading that Page's 1000 Views in order: 4.54 ms
            14909872  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 8.6 µs, p99 10.0 µs, max 90.3 µs
Page of 1000 Text Blocks, per Block added at the end: median 1173.0 µs, p99 2347.0 µs, max 2393.4 µs
reading that Page's 1000 Views in order: 4.65 ms
            14991792  peak memory footprint
```

Replica:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3104.7 µs, p99 6731.9 µs, max 13046.9 µs
reopening and reading that Page's 1001 Views: 15.4 ms; database with its log 2.1 MiB
            24658376  peak memory footprint
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3025.5 µs, p99 5937.4 µs, max 13087.7 µs
reopening and reading that Page's 1001 Views: 14.4 ms; database with its log 2.1 MiB
            24592840  peak memory footprint
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3074.2 µs, p99 6796.9 µs, max 10013.0 µs
reopening and reading that Page's 1001 Views: 16.4 ms; database with its log 2.1 MiB
            24199624  peak memory footprint
```
