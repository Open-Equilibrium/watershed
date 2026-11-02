# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- Observed on 2026-11-02 at commit `c10dca4`; three runs of each example. The `cbfd545` build of the pipeline example, run beside them, typed at a median of 10.5 µs per keystroke, so this session's machine was about 20 % slower than the earlier records below.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of all the Page's Views in order | the same |
| Durable typing | `/usr/bin/time -l target/release/examples/replica_observe` after `cargo build --release -p liquid-storage --example replica_observe` | 2,000 keystrokes appended to one Text Block in an encrypted replica, each its own Action and durable commit | Bridge and UI |
| Reopening a long Page | the same run | 1,000 more Text Blocks on that Page; then a new process opens the replica, finds the Page through its area and reads all its Views in order | the same |

## Observations

| Workload | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 10.6 µs, p99 42.3 µs, max 3.9 ms | median 9.9 µs, p99 15.2 µs, max 3.7 ms | median 10.0 µs, p99 15.1 µs, max 3.7 ms |
| Adding a Text Block at the end of the long Page | median 109 µs, p99 143 µs | median 107 µs, p99 131 µs | median 107 µs, p99 134 µs |
| Reading the long Page's 1,000 Views in order | 0.02 ms | 0.02 ms | 0.02 ms |
| Peak memory footprint of the pipeline process | 19.4 MB | 18.0 MB | 17.9 MB |
| Durable typing, per keystroke | median 3.05 ms, p99 6.03 ms, max 13.0 ms | median 3.06 ms, p99 6.12 ms, max 13.0 ms | median 3.05 ms, p99 6.08 ms, max 14.0 ms |
| A new process reopening the long Page and reading its 1,001 Views | 35.1 ms | 33.6 ms | 34.6 ms |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |

Each open Page keeps its View order as an index, so adding a Text Block at the end no longer reads every View: at `89700d9` it took a median of 1.19 ms and reading the order 4.56 ms on this Page. Readers take a window of the order rather than all of it.

A keystroke costs about what Automerge alone takes for it in the [CRDT spike](CRDT_SPIKE_EVIDENCE.md), 6–8 µs. The pipeline checks a text position against the elements beside it rather than reading the whole text, which took about 40 µs per keystroke in a 5,000-character Text Block.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). One run at `67b28e8`, before the replica turned it on, took a median of 72.1 µs per keystroke (raw output below). The editor may group consecutive keystrokes into one commit (client architecture rule 9; the approved History mockup shows one entry per pause), and typing is designed not to wait for a commit (rule 7).

Reopening is mostly Automerge loading the Page's 136 KB snapshot. Earlier records reopened inside the process that wrote the replica and measured 14.4 to 16.0 ms. How long the first load takes there depends on what that process did before: the same snapshot took 11.4 ms after the `cbfd545` build's writes, 29.5 ms after this build's, and about 12 ms on every later load in either. Only new-process runs are comparable. The 1,000 Text Blocks, each placed before the same neighbor, have sort keys of up to 204 characters, 104 KB in all: fractional indexing grows a key by about one character per five such placements.

### Raw output

Pipeline:

```text
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.6 µs, p99 42.3 µs, max 3936.9 µs
Page of 1000 Text Blocks, per Block added at the end: median 108.5 µs, p99 142.9 µs, max 1676.3 µs
reading that Page's 1000 Views in order: 0.02 ms
            19448264  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 9.9 µs, p99 15.2 µs, max 3664.3 µs
Page of 1000 Text Blocks, per Block added at the end: median 107.2 µs, p99 131.0 µs, max 1653.3 µs
reading that Page's 1000 Views in order: 0.02 ms
            18022832  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.0 µs, p99 15.1 µs, max 3677.6 µs
Page of 1000 Text Blocks, per Block added at the end: median 106.8 µs, p99 133.8 µs, max 1673.0 µs
reading that Page's 1000 Views in order: 0.02 ms
            17940912  peak memory footprint
```

Replica:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3053.0 µs, p99 6025.7 µs, max 12993.2 µs
database with its log after both: 2.1 MiB
a new process opening the replica and reading that Page's 1001 Views: 35.1 ms
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3061.6 µs, p99 6118.8 µs, max 13016.4 µs
database with its log after both: 2.1 MiB
a new process opening the replica and reading that Page's 1001 Views: 33.6 ms
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3050.7 µs, p99 6079.5 µs, max 14006.9 µs
database with its log after both: 2.1 MiB
a new process opening the replica and reading that Page's 1001 Views: 34.6 ms
```

Replica at `67b28e8`, without F_FULLFSYNC, one run:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 72.1 µs, p99 223.8 µs, max 1703.9 µs
reopening and reading that Page's 1001 Views: 14.8 ms; database with its log 2.1 MiB
            24166856  peak memory footprint
```
