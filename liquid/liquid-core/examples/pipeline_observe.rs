//! Observation workload for the Workspace core's mutation pipeline (PERFORMANCE.md, Liquid):
//! typing, Page building and a Database Block's rows through the pipeline, one recorded Action
//! per request, with an in-memory store. It prints a summary and never compares with a threshold. Run from the
//! repository root:
//!
//! ```sh
//! cargo build --release -p liquid-core --example pipeline_observe
//! /usr/bin/time -l target/release/examples/pipeline_observe
//! ```

use std::time::{Duration, Instant};

use liquid_core::{
    Actor, Cell, Created, FieldKind, Filter, Match, MemoryStore, Mutation, Origin, PageId,
    PersonId, Place, Request, RowQuery, Sort, Workspace,
};

const KEYSTROKES: usize = 10_000;
const BLOCKS: usize = 1_000;
const ROWS: usize = 5_000;

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
        .expect("the fixture creates an Area")
        .created
    else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    match workspace
        .apply(request(create))
        .expect("the fixture creates a Page")
        .created
    {
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
        times.last().expect("a summary has samples").as_secs_f64() * 1e6
    );
}

fn main() {
    // Typing: one Action per keystroke, the worst case for an editor that groups keystrokes.
    let mut workspace = Workspace::create(PersonId::new("observer"), MemoryStore::default())
        .expect("a new in-memory Workspace opens");
    let lisbon = page(&mut workspace);
    let add = Mutation::AddTextBlock {
        page: lisbon.clone(),
        after: None,
        text: String::new(),
    };
    let Some(Created::Block { block, .. }) = workspace
        .apply(request(add))
        .expect("the fixture creates a Block")
        .created
    else {
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
        workspace
            .apply(request(edit))
            .expect("the fixture edit is accepted");
        times.push(start.elapsed());
    }
    summary(
        &format!("typing, {KEYSTROKES} keystrokes into one Text Block, per keystroke"),
        times,
    );

    // A long Page: each Text Block added after the last one.
    let mut workspace = Workspace::create(PersonId::new("observer"), MemoryStore::default())
        .expect("a new in-memory Workspace opens");
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
        let created = workspace
            .apply(request(add))
            .expect("the fixture adds a Block")
            .created;
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
    let views = workspace
        .views(&lisbon, 0, usize::MAX)
        .expect("the Page reads its Views");
    println!(
        "reading that Page's {} Views in order: {:.2} ms",
        views.len(),
        start.elapsed().as_secs_f64() * 1e3
    );

    // A Database of 5,000 rows with four fields, as the CRDT spike measured, then one sorted
    // and filtered window as a Table View reads it.
    let mut workspace = Workspace::create(PersonId::new("observer"), MemoryStore::default())
        .expect("a new in-memory Workspace opens");
    let budget = page(&mut workspace);
    let add = Mutation::AddDatabaseBlock {
        page: budget.clone(),
        after: None,
        name: "Expenses".into(),
    };
    let Some(Created::Block { block, .. }) = workspace
        .apply(request(add))
        .expect("the fixture creates a Database Block")
        .created
    else {
        panic!("a Database")
    };
    let mut fields = vec![
        workspace
            .database(&budget, &block)
            .expect("the Database reads its Block")
            .expect("the Block has a Database")
            .fields[0]
            .id
            .clone(),
    ];
    for (name, kind) in [
        ("Amount", FieldKind::Number),
        ("Date", FieldKind::Date),
        ("Paid", FieldKind::Checkbox),
    ] {
        let add = Mutation::AddField {
            page: budget.clone(),
            block: block.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        };
        let Some(Created::Field(field)) = workspace
            .apply(request(add))
            .expect("the fixture creates a field")
            .created
        else {
            panic!("a field")
        };
        fields.push(field);
    }
    let (mut added, mut filled) = (Vec::with_capacity(ROWS), Vec::with_capacity(ROWS * 4));
    for i in 0..ROWS {
        let add = Mutation::AddRow {
            page: budget.clone(),
            block: block.clone(),
            place: Place::Last,
        };
        let start = Instant::now();
        let Some(Created::Row(row)) = workspace
            .apply(request(add))
            .expect("the fixture creates a row")
            .created
        else {
            panic!("a row")
        };
        added.push(start.elapsed());
        let values = [
            Cell::Text(format!("Item {i}")),
            Cell::Number((i * 37 % 1_000) as f64 + 0.5),
            Cell::Date(format!("2026-{:02}-{:02}", i % 12 + 1, i % 28 + 1)),
            Cell::Checkbox(i % 3 == 0),
        ];
        for (field, value) in fields.iter().zip(values) {
            let set = Mutation::SetCell {
                page: budget.clone(),
                block: block.clone(),
                row: row.clone(),
                field: field.clone(),
                value,
            };
            let start = Instant::now();
            workspace
                .apply(request(set))
                .expect("the fixture cell is set");
            filled.push(start.elapsed());
        }
    }
    summary(
        &format!("Database of {ROWS} rows, per row added at the end"),
        added,
    );
    summary("the same, per cell set", filled);
    let query = RowQuery {
        sort: Some(Sort {
            field: fields[1].clone(),
            descending: true,
        }),
        filter: Some(Filter {
            field: fields[3].clone(),
            matches: Match::Is(Cell::Checkbox(true)),
        }),
        start: 0,
        limit: 100,
    };
    let start = Instant::now();
    let window = workspace
        .rows(&budget, &block, &query)
        .expect("the Database reads its rows");
    println!(
        "a window of {} of its {} paid rows, sorted by amount: {:.2} ms",
        window.rows.len(),
        window.count,
        start.elapsed().as_secs_f64() * 1e3
    );
}
