# Liquid CRDT spike evidence (ADR-0238)

Acceptance evidence for [ADR-0238](../../docs/adr/ADR-LOG.md): Automerge against Loro on Liquid's merge cases, under the [evidence convention](../../PERFORMANCE.md#evidence-convention). No value here is a threshold; compare only like-for-like runs. The spike lives in [`liquid/spikes/crdt`](../spikes/crdt), outside the product workspace.

## Environment

- Apple M6, 24 GB, macOS 27.0; Rust 1.99.0, release build.
- Rust crates `automerge` 0.12.0 and `loro` 1.17.1, the current releases; Loro with `set_change_merge_interval(-1)` and text formatting that expands after its end, as Automerge's does.
- Observed on 2026-11-01 at commit `3299b63`; three runs.

## Convergence

`cargo test --test convergence` in `liquid/spikes/crdt` runs eight cases on both libraries; all 16 pass. Each case forks one seeded Page into replicas, edits them concurrently, syncs them both ways and requires identical canonical state:

| Case | Automerge | Loro |
| --- | --- | --- |
| Concurrent typing in one Text Block | both edits kept | both edits kept |
| Bold and concurrent typing | converges | converges |
| Two devices change one Database cell | **both values kept** | one value kept |
| Two devices change different fields of one row | both kept | both kept |
| Concurrent moves of the same View | each View once, same order | each View once, same order |
| Deletion cascade against a concurrent edit and a new Connection to the deleted Block | Block, View, Connection and own Automation gone, Workspace Automation off; the new Connection dangles on both replicas alike | same |
| Save and load | unchanged | unchanged |
| A Role denies one change; two replicas receive it directly and through a relay | both refuse the same two changes and converge | same |

## Workloads

| Workload | Command | Input | Exclusions |
| --- | --- | --- | --- |
| Typing | `cargo run --release` in `liquid/spikes/crdt` | 10,000 one-character inserts into one Text Block, one commit each, the worst case for an editor that groups keystrokes | Rendering, the bridge, disk |
| Large Page | same | 1,000 Text Blocks of 70 characters and 5,000 rows of four fields, one commit per Block or row; heap counted by a counting allocator, then saved and loaded | Disk |
| Role check on the large Page | same | 20 received text edits, each checked before it is applied | Signature checks |
| Sync burst | same | 1,000 committed remote edits applied at once | Network |
| Role check | same | 200 received text edits on a small Page | Signature checks |

## Observations

Typing stays far inside a frame on both libraries: Automerge 5.4 to 7.0 µs median, Loro 0.5 µs. Automerge holds the large Page in 1.9 MiB and saves it in 44 KiB; Loro needs 11.2 MiB after building it and 414 KiB saved, but loads it lazily into 606 KiB. Loro is about ten times faster on building, loading and the sync burst.

The Role check decides how the check must be built. Trying each received change on a fork of the Page and diffing it took 9.2 ms per change on the large Page with Automerge and 1.0 ms with Loro, because a fork copies the Page. Reading the change's own operations and asking whether the written objects lie under a denied object, as the spike's Automerge replica now does, took 27 to 31 µs.

## Findings for the implementation

1. Create a Page's top-level structure once, in the Page's first change. Two replicas that create the same key concurrently produce two objects, and Automerge shows only one of them.
2. Check received changes by their operations, never by trying them on a fork.
3. A denied change also refuses every later change of the same device on that Page, since each depends on its predecessor. The device must undo it and send its later work again, as ADR-0224's "refused change undone on its device" already requires.
4. A Connection created against a concurrently deleted Block dangles identically on every replica; Liquid shows it as needing attention by one deterministic read rule.
5. Loro merges a device's commits within 1,000 s into one change by default; per-change Role checks need that merging off.

Not yet observed: the other Liquid targets in [`PLATFORMS.md`](../../PLATFORMS.md) and the libraries inside the Flutter app. They follow with `liquid-core`, whose CI runs on Linux and macOS.

### Raw output

```text
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 7.0 µs, p99 15.2 µs, max 124.5 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 282 ms, heap 1.9 MiB; saved 44 KiB; loaded in 20 ms, heap 1.8 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 31.4 µs per change |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.0 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 13.9 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.6 µs, max 63.0 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 11 ms, heap 11.2 MiB; saved 414 KiB; loaded in 0 ms, heap 606 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 994.3 µs per change |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 28.4 µs per change |
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 5.4 µs, p99 6.3 µs, max 93.0 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 283 ms, heap 1.9 MiB; saved 44 KiB; loaded in 20 ms, heap 1.8 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 27.0 µs per change |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.0 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 14.0 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.6 µs, max 18.8 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 11 ms, heap 11.2 MiB; saved 414 KiB; loaded in 0 ms, heap 606 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 990.2 µs per change |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 28.1 µs per change |
| Library | Workload | Observation |
|---|---|---|
| Automerge 0.12.0 | typing, 10000 keystrokes, 1 commit each | median 5.4 µs, p99 6.2 µs, max 99.1 µs per keystroke; heap 1.7 MiB; saved 0 KiB |
| Automerge 0.12.0 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 283 ms, heap 1.9 MiB; saved 44 KiB; loaded in 20 ms, heap 1.8 MiB |
| Automerge 0.12.0 | Role-checked receive on that Page, 20 changes | 27.2 µs per change |
| Automerge 0.12.0 | sync burst, 1000 remote edits | applied in 2.0 ms |
| Automerge 0.12.0 | Role-checked receive, 200 changes | 14.3 µs per change |
| Loro 1.17.1 | typing, 10000 keystrokes, 1 commit each | median 0.5 µs, p99 0.6 µs, max 16.3 µs per keystroke; heap 2.1 MiB; saved 3 KiB |
| Loro 1.17.1 | Page, 1000 Text Blocks + 5000 rows × 4 fields | built in 11 ms, heap 11.2 MiB; saved 414 KiB; loaded in 0 ms, heap 606 KiB |
| Loro 1.17.1 | Role-checked receive on that Page, 20 changes | 987.7 µs per change |
| Loro 1.17.1 | sync burst, 1000 remote edits | applied in 0.1 ms |
| Loro 1.17.1 | Role-checked receive, 200 changes | 27.9 µs per change |
```
