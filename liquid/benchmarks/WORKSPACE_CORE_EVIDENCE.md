# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- Observed on 2026-11-02 at commit `e095071`, three rounds; each round also ran the pipeline example as built at `cbfd545`, before the View order index, for a like-for-like comparison.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action, with the in-memory store | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of all the Page's Views in order | the same |
| Durable typing | `/usr/bin/time -l target/release/examples/replica_observe` after `cargo build --release -p liquid-storage --example replica_observe` | 2,000 keystrokes appended to one Text Block in an encrypted replica, each its own Action and durable commit; then 1,000 Text Blocks on that Page | Bridge and UI |
| Reopening a long Page | `/usr/bin/time -l target/release/examples/replica_observe <path>` with the path that run printed | A new process opens the replica, finds the Page through its area and reads all its 1,001 Views in order | the same |

## Observations

| Workload | Round 1 | Round 2 | Round 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 10.5 µs, p99 42.4 µs, max 3.7 ms | median 10.1 µs, p99 12.1 µs, max 3.7 ms | median 10.1 µs, p99 11.9 µs, max 3.7 ms |
| … at `cbfd545` | median 10.1 µs, p99 12.5 µs, max 3.7 ms | median 10.1 µs, p99 14.6 µs, max 3.7 ms | median 10.0 µs, p99 13.1 µs, max 3.8 ms |
| Adding a Text Block at the end of the long Page | median 109 µs, p99 136 µs | median 108 µs, p99 132 µs | median 108 µs, p99 134 µs |
| … at `cbfd545` | median 1.17 ms, p99 2.30 ms | median 1.21 ms, p99 2.30 ms | median 1.17 ms, p99 2.28 ms |
| Reading the long Page's 1,000 Views in order | 0.02 ms | 0.02 ms | 0.02 ms |
| … at `cbfd545` | 4.43 ms | 4.39 ms | 4.37 ms |
| Peak memory footprint of the pipeline process | 19.4 MB | 17.8 MB | 17.8 MB |
| … at `cbfd545` | 18.2 MB | 18.3 MB | 18.2 MB |
| Durable typing, per keystroke | median 3.06 ms, p99 6.01 ms, max 14.0 ms | median 3.01 ms, p99 4.69 ms, max 10.7 ms | median 3.02 ms, p99 5.97 ms, max 11.7 ms |
| Peak memory footprint of the writing process | 11.1 MB | 10.9 MB | 11.2 MB |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |
| A new process reopening the long Page and reading its Views | 25.7 ms | 23.6 ms | 23.4 ms |
| Peak memory footprint of the reopening process | 19.1 MB | 19.1 MB | 19.1 MB |

Each loaded Page keeps its View order as an index, so adding a Text Block at the end no longer reads every View, and readers take a window of the order rather than all of it. Typing and memory stay as they were.

A keystroke through the pipeline with the in-memory store costs about 10 µs; without a store, at `89700d9`, it cost 8.6 µs, and Automerge alone 6–8 µs in the [CRDT spike](CRDT_SPIKE_EVIDENCE.md). The 3.7 ms maximum is the snapshot the pipeline writes after every 1,000 chunks of a document. The pipeline checks a text position against the elements beside it rather than reading the whole text, which took about 40 µs per keystroke in a 5,000-character Text Block.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). One run at `67b28e8`, before the replica turned it on, took a median of 72.1 µs per keystroke (raw output below). The editor may group consecutive keystrokes into one commit (client architecture rule 9; the approved History mockup shows one entry per pause), and typing is designed not to wait for a commit (rule 7).

Reopening is mostly Automerge loading the Page's snapshot. Earlier records reopened inside the process that had written the replica and measured 14.4 to 16.0 ms; a first load's time there depends on that process's earlier work, so only new-process runs are comparable (probe below).

### Probe of the long Page's snapshot

Once each at `6c06f0b` and at `cbfd545`, a modified `replica_observe` loaded the Page's stored chunks with `AutoCommit::load` after the writes, and read every View's sort key:

```text
6c06f0b: chunk 0: 136386 bytes, alone loads in 29.517583ms, changes 3001
cbfd545: chunk 0: 136277 bytes, alone loads in 11.423917ms, changes 3001
6c06f0b: branch binary loads branch snapshot in 12.090292ms (then 12.5 ms, 13.7 ms)
6c06f0b: branch binary loads main snapshot in 11.450084ms (then 10.9 ms, 10.9 ms)
6c06f0b and cbfd545 alike: views 1001 max sort 204 total 104304
```

Each of the 1,000 Text Blocks is placed after the previous one and before the typing Block's View, so fractional indexing lengthens their sort keys by about one character per five placements: the longest has 204 characters, and all keys together take 104 KB.

### Raw output

Pipeline at `e095071`, then at `cbfd545`, per round:

```text
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.5 µs, p99 42.4 µs, max 3696.8 µs
Page of 1000 Text Blocks, per Block added at the end: median 108.6 µs, p99 135.5 µs, max 1849.1 µs
reading that Page's 1000 Views in order: 0.02 ms
            19415496  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 12.5 µs, max 3686.6 µs
Page of 1000 Text Blocks, per Block added at the end: median 1167.9 µs, p99 2299.6 µs, max 3864.0 µs
reading that Page's 1000 Views in order: 4.43 ms
            18219440  peak memory footprint

typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 12.1 µs, max 3681.0 µs
Page of 1000 Text Blocks, per Block added at the end: median 108.2 µs, p99 132.3 µs, max 1703.5 µs
reading that Page's 1000 Views in order: 0.02 ms
            17793456  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 14.6 µs, max 3736.1 µs
Page of 1000 Text Blocks, per Block added at the end: median 1209.6 µs, p99 2302.1 µs, max 3927.2 µs
reading that Page's 1000 Views in order: 4.39 ms
            18252208  peak memory footprint

typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 11.9 µs, max 3685.2 µs
Page of 1000 Text Blocks, per Block added at the end: median 107.5 µs, p99 134.1 µs, max 1721.0 µs
reading that Page's 1000 Views in order: 0.02 ms
            17793456  peak memory footprint
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.0 µs, p99 13.1 µs, max 3758.7 µs
Page of 1000 Text Blocks, per Block added at the end: median 1173.3 µs, p99 2281.8 µs, max 3854.7 µs
reading that Page's 1000 Views in order: 4.37 ms
            18170288  peak memory footprint
```

Replica at `e095071`, writing and then reopening, per round:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 3057.0 µs, p99 6013.1 µs, max 13991.9 µs
database with its log after both: 2.1 MiB
            11059680  peak memory footprint
a new process opening the replica and reading that Page's 1001 Views: 25.7 ms
            19120584  peak memory footprint

typing, 2000 keystrokes, one durable commit each, per keystroke: median 3012.2 µs, p99 4687.5 µs, max 10746.6 µs
database with its log after both: 2.1 MiB
            10928584  peak memory footprint
a new process opening the replica and reading that Page's 1001 Views: 23.6 ms
            19071408  peak memory footprint

typing, 2000 keystrokes, one durable commit each, per keystroke: median 3019.8 µs, p99 5966.0 µs, max 11652.5 µs
database with its log after both: 2.1 MiB
            11157984  peak memory footprint
a new process opening the replica and reading that Page's 1001 Views: 23.4 ms
            19087792  peak memory footprint
```

Replica at `67b28e8`, without F_FULLFSYNC, one run:

```text
typing, 2000 keystrokes, one durable commit each, per keystroke: median 72.1 µs, p99 223.8 µs, max 1703.9 µs
reopening and reading that Page's 1001 Views: 14.8 ms; database with its log 2.1 MiB
            24166856  peak memory footprint
```
