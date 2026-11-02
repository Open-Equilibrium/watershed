//! Observation workload for the Workspace core's mutation pipeline (PERFORMANCE.md, Liquid):
//! typing and Page building through the pipeline, one recorded Action per request, with an
//! in-memory store. It prints a summary and never compares with a threshold. Run from the
//! repository root:
//!
//! ```sh
//! cargo build --release -p liquid-core --example pipeline_observe
//! /usr/bin/time -l target/release/examples/pipeline_observe
//! ```

use std::time::{Duration, Instant};

use liquid_core::{
    Actor, Created, MemoryStore, Mutation, Origin, PageId, PersonId, Request, Workspace,
};

const KEYSTROKES: usize = 10_000;
const BLOCKS: usize = 1_000;

fn request(mutation: Mutation) -> Request {
    Request {
        actor: Actor::Person(PersonId::new("observer")),
        origin: Origin::Ui,
        mutation,
    }
}

fn page(workspace: &mut Workspace<MemoryStore>) -> PageId {
    let Some(Created::Area(area)) = workspace
        .apply(request(Mutation::CreateArea {
            name: "Trips".into(),
        }))
        .unwrap()
        .created
    else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    match workspace.apply(request(create)).unwrap().created {
        Some(Created::Page(page)) => page,
        _ => panic!("a Page"),
    }
}

fn summary(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let at = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    println!(
        "{name}: median {:.1} µs, p99 {:.1} µs, max {:.1} µs",
        at(0.5).as_secs_f64() * 1e6,
        at(0.99).as_secs_f64() * 1e6,
        times.last().unwrap().as_secs_f64() * 1e6
    );
}

fn main() {
    // Typing: one Action per keystroke, the worst case for an editor that groups keystrokes.
    let mut workspace =
        Workspace::create(PersonId::new("observer"), MemoryStore::default()).unwrap();
    let lisbon = page(&mut workspace);
    let add = Mutation::AddTextBlock {
        page: lisbon.clone(),
        after: None,
        text: String::new(),
    };
    let Some(Created::Block { block, .. }) = workspace.apply(request(add)).unwrap().created else {
        panic!("a Block")
    };
    let mut times = Vec::with_capacity(KEYSTROKES);
    for i in 0..KEYSTROKES {
        let edit = Mutation::EditText {
            page: lisbon.clone(),
            block: block.clone(),
            at: i,
            delete: 0,
            insert: if i % 7 == 6 { " " } else { "x" }.into(),
        };
        let start = Instant::now();
        workspace.apply(request(edit)).unwrap();
        times.push(start.elapsed());
    }
    summary(
        &format!("typing, {KEYSTROKES} keystrokes into one Text Block, per keystroke"),
        times,
    );

    // A long Page: each Text Block added after the last one.
    let mut workspace =
        Workspace::create(PersonId::new("observer"), MemoryStore::default()).unwrap();
    let lisbon = page(&mut workspace);
    let mut after = None;
    let mut times = Vec::with_capacity(BLOCKS);
    for _ in 0..BLOCKS {
        let add = Mutation::AddTextBlock {
            page: lisbon.clone(),
            after: after.clone(),
            text: "Five days in Lisbon, one in Porto.".into(),
        };
        let start = Instant::now();
        let created = workspace.apply(request(add)).unwrap().created;
        times.push(start.elapsed());
        if let Some(Created::Block { view, .. }) = created {
            after = Some(view);
        }
    }
    summary(
        &format!("Page of {BLOCKS} Text Blocks, per Block added at the end"),
        times,
    );
    let start = Instant::now();
    let views = workspace.views(&lisbon).unwrap();
    println!(
        "reading that Page's {} Views in order: {:.2} ms",
        views.len(),
        start.elapsed().as_secs_f64() * 1e3
    );
}
