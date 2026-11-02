//! Observation workload for the local replica (PERFORMANCE.md, Liquid): typing with one
//! durable commit per keystroke, then a new process reopening a long Page from the encrypted
//! database. It
//! prints a summary and never compares with a threshold. Run from the repository root:
//!
//! ```sh
//! cargo build --release -p liquid-storage --example replica_observe
//! /usr/bin/time -l target/release/examples/replica_observe
//! ```

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use liquid_core::{Actor, Created, Mutation, Origin, PageId, PersonId, Request, Workspace};
use liquid_storage::Sqlite;

const KEYSTROKES: usize = 2_000;
const BLOCKS: usize = 1_000;
const KEY: [u8; 32] = [7; 32];

fn request(mutation: Mutation) -> Request {
    Request {
        actor: Actor::Person(PersonId::new("observer")),
        origin: Origin::Ui,
        mutation,
    }
}

fn created(workspace: &mut Workspace<Sqlite>, mutation: Mutation) -> Created {
    workspace
        .apply(request(mutation))
        .unwrap()
        .created
        .expect("a created object")
}

fn page(workspace: &mut Workspace<Sqlite>) -> PageId {
    let Created::Area(area) = created(
        workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    ) else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    match created(workspace, create) {
        Created::Page(page) => page,
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
    if let Some(path) = std::env::args().nth(1) {
        reopen(Path::new(&path));
        return;
    }
    let dir = std::env::temp_dir().join(format!("liquid-replica-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("replica.db");

    // Typing: one durable commit per keystroke, the worst case for an editor that groups
    // keystrokes into one commit.
    let mut workspace = Workspace::create(
        PersonId::new("observer"),
        Sqlite::open(&path, &KEY).unwrap(),
    )
    .unwrap();
    let lisbon = page(&mut workspace);
    let add = Mutation::AddTextBlock {
        page: lisbon.clone(),
        after: None,
        text: String::new(),
    };
    let Created::Block { block, .. } = created(&mut workspace, add) else {
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
        &format!("typing, {KEYSTROKES} keystrokes, one durable commit each, per keystroke"),
        times,
    );

    // A long Page, then a new session that opens it.
    let mut after = None;
    for _ in 0..BLOCKS {
        let add = Mutation::AddTextBlock {
            page: lisbon.clone(),
            after: after.clone(),
            text: "Five days in Lisbon, one in Porto.".into(),
        };
        if let Created::Block { view, .. } = created(&mut workspace, add) {
            after = Some(view);
        }
    }
    drop(workspace);
    let size: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    println!(
        "database with its log after both: {:.1} MiB",
        size as f64 / 1_048_576.0
    );
    // A new session in a new process, as when Liquid starts again.
    let reopened = Command::new(std::env::current_exe().unwrap())
        .arg(&path)
        .status()
        .unwrap();
    assert!(reopened.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Opens the replica at `path` and reads its first Page's Views, as the Explorer opens it.
fn reopen(path: &Path) {
    let start = Instant::now();
    let mut workspace = Workspace::open(Sqlite::open(path, &KEY).unwrap()).unwrap();
    let area = workspace.areas().unwrap().remove(0).id;
    let page = workspace.pages(&area).unwrap().remove(0).id;
    let views = workspace.views(&page, 0, usize::MAX).unwrap();
    println!(
        "a new process opening the replica and reading that Page's {} Views: {:.1} ms",
        views.len(),
        start.elapsed().as_secs_f64() * 1e3
    );
}
