# Liquid CRDT spike evidence (ADR-0238)

Acceptance evidence for [ADR-0238](../../docs/adr/ADR-LOG.md): Automerge against Loro on Liquid's merge cases, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs. The spike lives in [`liquid/spikes/crdt`](../spikes/crdt), outside the product workspace.

## Environment

- Apple M6, 24 GB, macOS 27.0; Rust 1.99.0, release build.
- Rust crates `automerge` 0.12.0 and `loro` 1.17.1, the current releases; Loro with `set_change_merge_interval(-1)` and text formatting that expands after its end, as Automerge's does.
- Observed on 2026-11-02 at commit `b3f1c47`; three runs.

## Convergence

`cargo test --test convergence` in `liquid/spikes/crdt` runs ten cases on both libraries; all 20 pass. Each case forks one seeded Page into replicas, edits them concurrently, syncs them both ways and requires identical canonical state:

| Case | Automerge | Loro |
| --- | --- | --- |
| Concurrent typing in one Text Block | both edits kept | both edits kept |
| Bold and concurrent typing | converges | converges |
| Two devices change one Database cell | **both values kept** | one value kept |
| Two devices change different fields of one row | both kept | both kept |
| Concurrent moves of the same View | each View once, same order | each View once, same order |
| A move concurrent with the Block's deletion | the View stays deleted | the View stays deleted |
| Deletion cascade against a concurrent edit and a new Connection to the deleted Block | Block, View, Connection and own Automation gone, Workspace Automation off; the new Connection dangles on both replicas alike | same |
| Save and load | unchanged | unchanged |
| A Role denies one change; two replicas receive it directly and through a relay | both refuse the same two changes and converge | same |
| A denied cell write and an allowed concurrent write to the same cell, received in either order | both replicas refuse the denied write and keep the allowed value | same |

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `cargo run --release` in `liquid/spikes/crdt` | 10,000 one-character inserts into one Text Block, one commit each, the worst case for an editor that groups keystrokes | Rendering, the bridge, disk |
| Large Page | same | 1,000 Text Blocks of 70 characters and 5,000 rows of four fields, one commit per Block or row; heap counted by a counting allocator, then saved and loaded | Disk |
| Role check on the large Page | same | 20 received text edits from a device that may not change rows, each checked before it is applied, after a first check that indexes the Page's history | Signature checks |
| Sync burst | same | 1,000 committed remote edits applied at once | Network |
| Role check | same | 200 received text edits on a small Page from a device that may not change rows | Signature checks |

## Observations

Typing stays far inside a frame on both libraries: Automerge 6.0 to 8.0 µs median, Loro 0.5 µs. Automerge holds the large Page in 2.0 MiB and saves it in 48 KiB; Loro needs 11.8 MiB after building it and 448 KiB saved, but loads it lazily into 744 KiB. Loro is about ten times faster on building, loading and the sync burst.

The Role check decides how the check must be built. Trying each received change on a fork of the Page and diffing it, as the first version of the spike did, took 9.2 ms per change on the large Page with Automerge and 1.0 ms with Loro, because a fork copies the Page, and its answer depended on which concurrent write was visible. Reading the change's own operations against an index of where each object was created takes 27 to 29 µs per change with Automerge and 16 to 21 µs with Loro on the large Page, after indexing the Page's history once in 26 ms and 4 ms.

## Findings for the implementation

1. Create a Page's top-level structure once, in the Page's first change. Two replicas that create the same key concurrently produce two objects, and Automerge shows only one of them.
2. Check received changes by their operations against an index of where each object was created, kept with the replica; never by the visible state or by trying them on a fork, which depends on which concurrent write won.
3. Give every View its own object and move it by writing inside it, so a concurrent deletion of the View wins instead of the move bringing it back.
4. A denied change also refuses every later change of the same device on that Page, since each depends on its predecessor. The device must undo it and send its later work again, as ADR-0224's "refused change undone on its device" already requires.
5. A Connection created against a concurrently deleted Block dangles identically on every replica; Liquid shows it as needing attention by one deterministic read rule.
6. Loro merges a device's commits within 1,000 s into one change by default; per-change Role checks need that merging off.

Not yet observed: the other Liquid targets in [`PLATFORMS.md`](../../PLATFORMS.md) and the libraries inside the Flutter app. They follow with `liquid-core`, whose CI runs on Linux and macOS.

### Raw output

```text
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 8.0 µs, p99 25.8 µs, max 126.9 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 288 ms, heap 2.0 MiB; saved 48 KiB; loaded in 20 ms, heap 2.0 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 29.4 µs per change, after indexing the Page's history once in 26 ms |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.0 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 15.1 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.6 µs, max 63.3 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 11 ms, heap 11.8 MiB; saved 448 KiB; loaded in 0 ms, heap 744 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 21.4 µs per change, after indexing the Page's history once in 4 ms |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 4.5 µs per change |
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 6.0 µs, p99 6.8 µs, max 103.3 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 288 ms, heap 2.0 MiB; saved 48 KiB; loaded in 20 ms, heap 2.0 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 27.8 µs per change, after indexing the Page's history once in 26 ms |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.1 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 15.9 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.7 µs, max 17.5 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 12 ms, heap 11.8 MiB; saved 448 KiB; loaded in 0 ms, heap 744 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 15.9 µs per change, after indexing the Page's history once in 4 ms |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 4.8 µs per change |
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 6.0 µs, p99 6.8 µs, max 95.8 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 289 ms, heap 2.0 MiB; saved 48 KiB; loaded in 20 ms, heap 2.0 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 27.3 µs per change, after indexing the Page's history once in 26 ms |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.0 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 15.9 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.7 µs, max 17.7 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 11 ms, heap 11.8 MiB; saved 448 KiB; loaded in 0 ms, heap 744 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 16.6 µs per change, after indexing the Page's history once in 4 ms |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 4.7 µs per change |
```
