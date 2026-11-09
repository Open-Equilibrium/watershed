# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs.

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- Observed on 2026-11-02 at commit `e095071`, three rounds. The Database workload was observed on 2026-11-03 at `6aa3f03`, three rounds, and after its workload change on 2026-11-06 at `e4193e5`, three rounds, with rows added last (ADR-0247).

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action, with the in-memory store | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of all the Page's Views in order | the same |
| Database | the same run | A Database Block of 5,000 rows with a text, number, date and checkbox field, each field and row added last (ADR-0247; at `6aa3f03` after the last one) and each cell set as its own request; then one window of 100 rows, filtered by the checkbox and sorted by the number | the same |
| Durable typing | `/usr/bin/time -l target/release/examples/replica_observe` after `cargo build --release -p liquid-storage --example replica_observe` | 2,000 keystrokes appended to one Text Block in an encrypted replica, each its own Action and durable commit; then 1,000 Text Blocks on that Page | Bridge and UI |
| Reopening a long Page | `/usr/bin/time -l target/release/examples/replica_observe <path>` with the path that run printed | A new process opens the replica, finds the Page through its area and reads all its 1,001 Views in order | the same |

## Observations

| Workload | Round 1 | Round 2 | Round 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 10.5 µs, p99 42.4 µs, max 3.7 ms | median 10.1 µs, p99 12.1 µs, max 3.7 ms | median 10.1 µs, p99 11.9 µs, max 3.7 ms |
| Adding a Text Block at the end of the long Page | median 109 µs, p99 136 µs | median 108 µs, p99 132 µs | median 108 µs, p99 134 µs |
| Reading the long Page's 1,000 Views in order | 0.02 ms | 0.02 ms | 0.02 ms |
| Peak memory footprint of the pipeline process | 19.4 MB | 17.8 MB | 17.8 MB |
| Adding a Database row at the end, before the workload change (2026-11-03, `6aa3f03`) | median 55.7 µs, p99 90.2 µs, max 13.0 ms | median 53.7 µs, p99 82.5 µs, max 12.7 ms | median 52.7 µs, p99 85.6 µs, max 12.6 ms |
| … added last (2026-11-06, `e4193e5`) | median 47.8 µs, p99 59.1 µs, max 12.7 ms | median 49.1 µs, p99 63.8 µs, max 12.7 ms | median 48.2 µs, p99 60.5 µs, max 12.4 ms |
| Setting one cell | median 36.9 µs, p99 60.5 µs | median 34.8 µs, p99 50.5 µs | median 34.7 µs, p99 52.7 µs |
| A filtered, sorted window of 100 of the 5,000 rows | 0.81 ms | 0.82 ms | 0.88 ms |
| Peak memory footprint of the pipeline process, with the Database | 101.0 MB | 101.2 MB | 100.5 MB |
| Durable typing, per keystroke | median 3.06 ms, p99 6.01 ms, max 14.0 ms | median 3.01 ms, p99 4.69 ms, max 10.7 ms | median 3.02 ms, p99 5.97 ms, max 11.7 ms |
| Peak memory footprint of the writing process | 11.1 MB | 10.9 MB | 11.2 MB |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |
| A new process reopening the long Page and reading its Views | 25.7 ms | 23.6 ms | 23.4 ms |
| Peak memory footprint of the reopening process | 19.1 MB | 19.1 MB | 19.1 MB |

Each loaded Database Block keeps its rows with their cells as an index beside the Page's document, as the View order is kept, so adding a row does not read every row and a window is filtered and sorted from memory (client architecture rule 13). The maximum while adding rows is the snapshot written after every 1,000 chunks, which grows with the Database. With the Database, the pipeline process holds 25,000 more Actions with their changes in the in-memory store, which accounts for most of its larger footprint.

Each loaded Page keeps its View order as an index, so adding a Text Block at the end does not read every View, and readers take a window of the order rather than all of it.

A keystroke through the pipeline with the in-memory store costs about 10 µs. The 3.7 ms maximum is the snapshot the pipeline writes after every 1,000 chunks of a document. The pipeline checks a text position against the elements beside it rather than reading the whole text, which took about 40 µs per keystroke in a 5,000-character Text Block.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). The editor may group consecutive keystrokes into one commit (client architecture rule 9; the approved History mockup shows one entry per pause), and typing is designed not to wait for a commit (rule 7).

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

Pipeline at `6aa3f03`, the Database part, per round:

```text
Database of 5000 rows, per row added at the end: median 55.7 µs, p99 90.2 µs, max 12991.1 µs
the same, per cell set: median 36.9 µs, p99 60.5 µs, max 319.7 µs
a window of 100 of its 1667 paid rows, sorted by amount: 0.81 ms
           100991624  peak memory footprint

Database of 5000 rows, per row added at the end: median 53.7 µs, p99 82.5 µs, max 12705.5 µs
the same, per cell set: median 34.8 µs, p99 50.5 µs, max 188.9 µs
a window of 100 of its 1667 paid rows, sorted by amount: 0.82 ms
           101204592  peak memory footprint

Database of 5000 rows, per row added at the end: median 52.7 µs, p99 85.6 µs, max 12604.3 µs
the same, per cell set: median 34.7 µs, p99 52.7 µs, max 185.5 µs
a window of 100 of its 1667 paid rows, sorted by amount: 0.88 ms
           100532848  peak memory footprint
```

Pipeline at `e095071`, per round:

```text
typing, 10000 keystrokes into one Text Block, per keystroke: median 10.5 µs, p99 42.4 µs, max 3696.8 µs
Page of 1000 Text Blocks, per Block added at the end: median 108.6 µs, p99 135.5 µs, max 1849.1 µs
reading that Page's 1000 Views in order: 0.02 ms
            19415496  peak memory footprint

typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 12.1 µs, max 3681.0 µs
Page of 1000 Text Blocks, per Block added at the end: median 108.2 µs, p99 132.3 µs, max 1703.5 µs
reading that Page's 1000 Views in order: 0.02 ms
            17793456  peak memory footprint

typing, 10000 keystrokes into one Text Block, per keystroke: median 10.1 µs, p99 11.9 µs, max 3685.2 µs
Page of 1000 Text Blocks, per Block added at the end: median 107.5 µs, p99 134.1 µs, max 1721.0 µs
reading that Page's 1000 Views in order: 0.02 ms
            17793456  peak memory footprint
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
