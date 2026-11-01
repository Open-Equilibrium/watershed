//! Fixed observational workloads for ADR-0238 (PERFORMANCE.md: observations, no thresholds).
//! Run with `cargo run --release` and copy the table into the evidence file.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use liquid_crdt_spike::{Policy, Replica, am::Am, lo::Lo};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn mib(bytes: usize) -> String {
    if bytes < 1_048_576 {
        format!("{:.0} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    }
}

fn micros(d: Duration) -> String {
    format!("{:.1} µs", d.as_secs_f64() * 1e6)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

const KEYSTROKES: usize = 10_000;
const BLOCKS: usize = 1_000;
const ROWS: usize = 5_000;
const BURST: usize = 1_000;
const CHECKED: usize = 200;
const LARGE_CHECKED: usize = 20;

fn run<R: Replica>(name: &str) {
    // Typing: one keystroke per commit, the worst case for an editor that groups keystrokes.
    let base = live();
    let mut seed = R::seed(9);
    seed.add_text_block("t", "", "a0");
    seed.commit();
    let mut doc = seed.fork_as(1);
    let mut times = Vec::with_capacity(KEYSTROKES);
    for i in 0..KEYSTROKES {
        let start = Instant::now();
        doc.edit_text("t", i, 0, if i % 7 == 6 { " " } else { "x" });
        doc.commit();
        times.push(start.elapsed());
    }
    times.sort();
    let typing_heap = live().saturating_sub(base);
    let typing_save = doc.save().len();
    println!(
        "| {name} | typing, {KEYSTROKES} keystrokes, 1 commit each | median {}, p99 {}, max {} per keystroke; heap {}; saved {} |",
        micros(percentile(&times, 0.5)),
        micros(percentile(&times, 0.99)),
        micros(*times.last().unwrap()),
        mib(typing_heap),
        mib(typing_save)
    );
    drop((doc, seed));

    // A large Page: Text Blocks and Database rows, one commit each.
    let base = live();
    let start = Instant::now();
    let mut page = R::seed(9);
    for i in 0..BLOCKS {
        page.add_text_block(
            &format!("b{i}"),
            &"Five days in Lisbon, one in Porto. ".repeat(2),
            &format!("k{i:05}"),
        );
        page.commit();
    }
    for i in 0..ROWS {
        let row = format!("r{i}");
        for (field, value) in [
            ("date", "27 Oct"),
            ("item", "Groceries"),
            ("category", "Food"),
            ("amount", "84.20"),
        ] {
            page.put_cell(&row, field, value);
        }
        page.commit();
    }
    let build = start.elapsed();
    let page_heap = live().saturating_sub(base);
    let bytes = page.save();
    // Role-checked receive on the large Page: each trial fork copies the Page.
    let (mut a, mut b) = (page.fork_as(1), page.fork_as(2));
    for i in 0..LARGE_CHECKED {
        b.edit_text("b0", i, 0, "z");
        b.commit();
    }
    let start = Instant::now();
    assert!(
        a.pull_checked(
            &mut b,
            Policy {
                denied_rows_peer: 3
            }
        )
        .is_empty()
    );
    let large_checked = start.elapsed() / LARGE_CHECKED as u32;
    drop((a, b, page));
    let base = live();
    let start = Instant::now();
    let loaded = R::load(&bytes);
    let load = start.elapsed();
    let loaded_heap = live().saturating_sub(base);
    println!(
        "| {name} | Page, {BLOCKS} Text Blocks + {ROWS} rows × 4 fields | built in {:.0} ms, heap {}; saved {}; loaded in {:.0} ms, heap {} |",
        build.as_secs_f64() * 1e3,
        mib(page_heap),
        mib(bytes.len()),
        load.as_secs_f64() * 1e3,
        mib(loaded_heap)
    );
    println!(
        "| {name} | Role-checked receive on that Page, {LARGE_CHECKED} changes | {} per change |",
        micros(large_checked)
    );
    drop(loaded);

    // A sync burst: one replica applies another's committed edits.
    let mut seed = R::seed(9);
    seed.add_text_block("t", "start", "a0");
    seed.commit();
    let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
    for i in 0..BURST {
        b.edit_text("t", 5 + i, 0, "y");
        b.commit();
    }
    let start = Instant::now();
    a.pull(&mut b);
    println!(
        "| {name} | sync burst, {BURST} remote edits | applied in {:.1} ms |",
        start.elapsed().as_secs_f64() * 1e3
    );

    // Role-checked receive: each change is tried on a fork and its diff inspected.
    let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
    for i in 0..CHECKED {
        b.edit_text("t", 5 + i, 0, "z");
        b.commit();
    }
    let start = Instant::now();
    let refused = a.pull_checked(
        &mut b,
        Policy {
            denied_rows_peer: 3,
        },
    );
    assert!(refused.is_empty());
    println!(
        "| {name} | Role-checked receive, {CHECKED} changes | {} per change |",
        micros(start.elapsed() / CHECKED as u32)
    );
}

fn main() {
    println!("| Library | Workload | Observation |\n|---|---|---|");
    run::<Am>("Automerge 0.12.0");
    run::<Lo>("Loro 1.17.1");
}
