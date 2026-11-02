# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- The pipeline observed on 2026-11-02 at commit `89700d9`, the replica at `e751819`; three runs each on an otherwise idle machine. A set of runs at the same commit while other builds ran took about 35 % longer and is not comparable.

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
| Durable typing, per keystroke | median 3.04 ms, p99 6.21 ms, max 12.8 ms | median 3.04 ms, p99 5.99 ms, max 14.3 ms | median 3.07 ms, p99 6.12 ms, max 12.1 ms |
| Reopening the long Page and reading its 1,001 Views | 14.9 ms | 14.4 ms | 16.0 ms |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |
| Peak memory footprint of the replica process | 25.3 MB | 24.8 MB | 24.7 MB |

A keystroke costs about what Automerge alone takes for it in the [CRDT spike](CRDT_SPIKE_EVIDENCE.md), 6–8 µs. The pipeline checks a text position against the elements beside it, not by reading the whole text; reading the text took about 40 µs per keystroke in a 5,000-character Text Block.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). One run at `67b28e8`, before the replica turned it on, took a median of 72.1 µs per keystroke (raw output below). The editor may group consecutive keystrokes into one commit (client architecture rule 9; the approved History mockup shows one entry per pause), and typing is designed not to wait for a commit (rule 7).

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
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3036.5 µs, p99 6206.7 µs, max 12825.2 µs
reopening and reading that Page's 1001 Views: 14.9 ms; database with its log 2.1 MiB
            25346504  peak memory footprint
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3035.8 µs, p99 5991.5 µs, max 14262.5 µs
reopening and reading that Page's 1001 Views: 14.4 ms; database with its log 2.1 MiB
            24789448  peak memory footprint
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3071.8 µs, p99 6119.5 µs, max 12069.7 µs
reopening and reading that Page's 1001 Views: 16.0 ms; database with its log 2.1 MiB
            24707552  peak memory footprint
```

Replica at `67b28e8`, without F_FULLFSYNC, one run:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 72.1 µs, p99 223.8 µs, max 1703.9 µs
reopening and reading that Page's 1001 Views: 14.8 ms; database with its log 2.1 MiB
            24166856  peak memory footprint
```
