# Liquid Workspace core evidence

Observational evidence for `liquid-core`'s mutation pipeline and `liquid-storage`'s local replica, kept under the [evidence convention](../../PERFORMANCE.md#evidence-convention).

## Environment

- Apple M6, 24 GB, macOS 27.0 (26A428), internal SSD; Rust 1.99.0, release build; `automerge` 0.12.0, `rusqlite` 0.41.1 with SQLCipher 4.19.0.
- Observed on 2026-11-02 at commit `e095071`, three rounds; the Typing and Long Page workloads again on 2026-11-22 at `9f2b3e01`, three rounds, after the Long Page workload added an arranged Page (ADR-0312). The Database workload was observed on 2026-11-03 at `6aa3f03`, three rounds, after its workload change on 2026-11-06 at `e4193e5`, three rounds, with rows added last (ADR-0247), and last on 2026-11-22 at `6de4f29d`, three rounds, with a Board's grouped read added (ADR-0311). The attachment workload was observed on 2026-11-17 at `1557b2a3`, three rounds, with `rusqlite` 0.42.0, and the Whiteboard workload on 2026-11-17 at `aa72dab6`, three rounds.

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `/usr/bin/time -l target/release/examples/pipeline_observe` after `cargo build --release -p liquid-core --example pipeline_observe` | 10,000 keystrokes appended to one Text Block, each its own request and Action, with the in-memory store | Durable commit, bridge and UI |
| Long Page | the same run | 1,000 Text Blocks, each added after the last; then one read of all the Page's Views in order; then its first Section given a tile and 1,000 more Text Blocks added, so every one is checked for a split Section | the same |
| Database | the same run | A Database Block of 5,000 rows with a text, number, date and checkbox field, each field and row added last (ADR-0247; at `6aa3f03` after the last one) and each cell set as its own request; then one window of 100 rows, filtered by the checkbox and sorted by the number, and a Board's read: the rows grouped by the checkbox, each group's first 50 sorted by the number | the same |
| Durable typing | `/usr/bin/time -l target/release/examples/replica_observe` after `cargo build --release -p liquid-storage --example replica_observe` | 2,000 keystrokes appended to one Text Block in an encrypted replica, each its own Action and durable commit; then 1,000 Text Blocks on that Page | Bridge and UI |
| Reopening a long Page | `/usr/bin/time -l target/release/examples/replica_observe <path>` with the path that run printed | A new process opens the replica, finds the Page through its area and reads all its 1,001 Views in order | the same |
| Attachment | `/usr/bin/time -l target/release/examples/replica_observe --attachment` | A 256 MiB file added to a new encrypted replica in 1 MiB parts, each its own durable commit, with its SHA-256; then read back part by part (ADR-0305) | Reading the file from disk, bridge and UI |
| Whiteboard | `/usr/bin/time -l target/release/examples/replica_observe --whiteboard` | 2,000 strokes of 150 points drawn on a Whiteboard in an encrypted replica, each its own edit and durable commit; the Page closed and the board read whole; then a second board filled to its cap of 1,000,000 points, 100 strokes of 10,000, and read whole (ADR-0306) | Outlining and drawing strokes, bridge and UI |

## Observations

| Workload | Round 1 | Round 2 | Round 3 |
| --- | --- | --- | --- |
| Typing, per keystroke | median 11.2 µs, p99 18.6 µs, max 3.8 ms | median 11.0 µs, p99 13.0 µs, max 3.8 ms | median 11.1 µs, p99 13.2 µs, max 3.8 ms |
| Adding a Text Block at the end of the long Page, before the workload change (2026-11-02, `e095071`) | median 109 µs, p99 136 µs | median 108 µs, p99 132 µs | median 108 µs, p99 134 µs |
| … after it (2026-11-22, `9f2b3e01`) | median 140 µs, p99 172 µs | median 141 µs, p99 176 µs | median 139 µs, p99 174 µs |
| … once the Page is arranged, up to 2,000 Text Blocks | median 195 µs, p99 250 µs | median 195 µs, p99 251 µs | median 195 µs, p99 247 µs |
| Reading the long Page's 1,000 Views in order | 0.03 ms | 0.02 ms | 0.02 ms |
| Peak memory footprint of the pipeline process | 19.4 MB | 17.8 MB | 17.8 MB |
| Adding a Database row at the end, before the workload change (2026-11-03, `6aa3f03`) | median 55.7 µs, p99 90.2 µs, max 13.0 ms | median 53.7 µs, p99 82.5 µs, max 12.7 ms | median 52.7 µs, p99 85.6 µs, max 12.6 ms |
| … added last (2026-11-22, `6de4f29d`) | median 51.3 µs, p99 64.4 µs, max 0.15 ms | median 49.7 µs, p99 59.0 µs, max 0.15 ms | median 50.0 µs, p99 61.3 µs, max 0.15 ms |
| Setting one cell | median 36.2 µs, p99 50.9 µs | median 35.9 µs, p99 50.4 µs | median 35.4 µs, p99 51.0 µs |
| A filtered, sorted window of 100 of the 5,000 rows | 0.87 ms | 0.81 ms | 0.85 ms |
| Both groups with their first 50 rows each, sorted | 1.86 ms | 1.80 ms | 1.84 ms |
| Peak memory footprint of the pipeline process, with the Database | 102.6 MB | 101.6 MB | 101.3 MB |
| Durable typing, per keystroke | median 3.06 ms, p99 6.01 ms, max 14.0 ms | median 3.01 ms, p99 4.69 ms, max 10.7 ms | median 3.02 ms, p99 5.97 ms, max 11.7 ms |
| Peak memory footprint of the writing process | 11.1 MB | 10.9 MB | 11.2 MB |
| Replica on disk with its write-ahead log | 2.1 MiB | 2.1 MiB | 2.1 MiB |
| A new process reopening the long Page and reading its Views | 25.7 ms | 23.6 ms | 23.4 ms |
| Peak memory footprint of the reopening process | 19.1 MB | 19.1 MB | 19.1 MB |
| Adding the 256 MiB file, per 1 MiB part | median 11.0 ms, p99 70.1 ms, max 90.0 ms | median 11.3 ms, p99 61.2 ms, max 75.0 ms | median 11.8 ms, p99 69.0 ms, max 76.2 ms |
| Adding it whole | 4.47 s, 57 MiB/s | 4.53 s, 56 MiB/s | 4.77 s, 54 MiB/s |
| Reading it back part by part | 0.27 s, 942 MiB/s | 0.26 s, 971 MiB/s | 0.27 s, 945 MiB/s |
| Peak memory footprint of the attachment process | 9.7 MB | 9.7 MB | 11.8 MB |
| Drawing a stroke of 150 points | median 6.14 ms, p99 10.0 ms, max 56.0 ms | median 6.09 ms, p99 9.72 ms, max 48.1 ms | median 6.94 ms, p99 10.1 ms, max 49.8 ms |
| Loading the Page and reading its 2,000 strokes | 22.3 ms | 22.3 ms | 22.2 ms |
| … a board at its point cap | 33.9 ms | 33.7 ms | 34.0 ms |
| Peak memory footprint of the Whiteboard process | 130.9 MB | 130.9 MB | 131.7 MB |

Each loaded Database Block keeps its rows with their cells as an index beside the Page's document, as the View order is kept, so adding a row does not read every row and a window is filtered and sorted from memory (client architecture rule 13). The maximum while adding rows is the snapshot written after every 1,000 chunks, which grows with the Database. With the Database, the pipeline process holds 25,000 more Actions with their changes in the in-memory store, which accounts for most of its larger footprint.

Each loaded Page keeps its View order as an index, so adding a Text Block at the end does not read every View, and readers take a window of the order rather than all of it.

A keystroke through the pipeline with the in-memory store costs about 10 µs. The 3.7 ms maximum is the snapshot the pipeline writes after every 1,000 chunks of a document. The pipeline checks a text position against the elements beside it rather than reading the whole text.

A durable commit is almost entirely the flush to the storage medium: F_FULLFSYNC, which macOS needs for a commit to survive a power loss (ADR-0237). The editor may group consecutive keystrokes into one commit (client architecture rule 9; the approved History mockup shows one entry per pause), and typing is designed not to wait for a commit (rule 7).

Adding an attachment costs about 11 ms per 1 MiB part; memory stays flat, as each part is written and dropped. Bridge calls take the Workspace's lock for one part at a time, so other writes wait at most one part's commit.

A stroke's edit is a durable commit of its points, about 1.8 KB here, and of a dozen values; it also counts the board's live elements and points against their caps, which a probe at `aa72dab6` saw add about 1 µs per element: about 6 ms per stroke for the first 200 strokes and 8 ms for the last 200. The maxima are the snapshots after every 1,000 chunks. The process's footprint is mostly the second board's 12 MB of points, held by the document and its recorded Actions; reading a board at its cap, the most one ever crosses to Dart, takes about 34 ms.

Reopening is mostly Automerge loading the Page's snapshot.

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

Replica at `aa72dab6`, the Whiteboard workload, per round:

```text
drawing 2000 strokes of 150 points, per stroke: median 6139.1 µs, p99 10034.9 µs, max 55972.6 µs
loading the Page and reading its 2000 strokes: 22.3 ms
loading the Page and reading a board at its cap, 100 strokes of 10000 points: 33.9 ms
           130908712  peak memory footprint

drawing 2000 strokes of 150 points, per stroke: median 6093.6 µs, p99 9719.0 µs, max 48088.3 µs
loading the Page and reading its 2000 strokes: 22.3 ms
loading the Page and reading a board at its cap, 100 strokes of 10000 points: 33.7 ms
           130859560  peak memory footprint

drawing 2000 strokes of 150 points, per stroke: median 6944.9 µs, p99 10063.8 µs, max 49816.1 µs
loading the Page and reading its 2000 strokes: 22.2 ms
loading the Page and reading a board at its cap, 100 strokes of 10000 points: 34.0 ms
           131662376  peak memory footprint
```
