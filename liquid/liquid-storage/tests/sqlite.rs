//! The local replica as a later session sees it: what was acknowledged is there after a
//! reopen, and the file reveals nothing without its key.

use std::path::{Path, PathBuf};

use liquid_core::{
    Actor, Created, Document, Error, Mutation, Operation, Origin, PageId, PersonId, Place, Request,
    Store, Workspace,
};
use liquid_storage::Sqlite;

const KEY: [u8; 32] = [7; 32];

/// A fresh database path that is removed with its write-ahead log when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "liquid-storage-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("replica.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

fn apply(workspace: &mut Workspace<Sqlite>, mutation: Mutation) -> Option<Created> {
    workspace
        .apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        })
        .unwrap()
        .created
}

/// A Workspace with one area and one Page holding the Text Block "Five days in Lisbon".
fn lisbon(path: &Path) -> (PageId, liquid_core::BlockId) {
    let store = Sqlite::open(path, &KEY).unwrap();
    let mut workspace = Workspace::create(PersonId::new("benni"), store).unwrap();
    let Some(Created::Area(area)) = apply(
        &mut workspace,
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
    let Some(Created::Page(page)) = apply(&mut workspace, create) else {
        panic!("a Page")
    };
    let add = Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days in Lisbon".into(),
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add) else {
        panic!("a Block")
    };
    (page, block)
}

#[test]
fn an_acknowledged_write_is_there_after_a_reopen() {
    let scratch = Scratch::new("reopen");
    let (page, block) = lisbon(&scratch.db());

    let mut workspace = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();
    let area = workspace.areas().unwrap().remove(0);
    assert_eq!(area.name, "Trips");
    assert_eq!(workspace.pages(&area.id).unwrap()[0].title, "Lisbon");
    let edit = Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    };
    apply(&mut workspace, edit);
    drop(workspace);

    let mut workspace = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();
    assert_eq!(
        workspace.text(&page, &block).unwrap().unwrap(),
        "Our Five days in Lisbon"
    );
    let history = workspace.history(None, 10).unwrap();
    assert_eq!(history.len(), 5);
    assert!(matches!(
        history[0].operation,
        Operation::Mutation(Mutation::EditText { .. })
    ));
    assert_eq!(history[4].operation, Operation::CreateWorkspace);
}

#[test]
fn history_reads_in_pages_newest_first() {
    let scratch = Scratch::new("history");
    lisbon(&scratch.db());
    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();

    let newest = store.history(None, 2).unwrap();
    let older = store.history(Some(&newest[1].id), 10).unwrap();
    assert_eq!(newest.len(), 2);
    assert_eq!(older.len(), 2);
    assert_eq!(older[1].operation, Operation::CreateWorkspace);
    let mut elsewhere =
        Workspace::create(PersonId::new("benni"), liquid_core::MemoryStore::default()).unwrap();
    let foreign = elsewhere.history(None, 1).unwrap().remove(0).id;
    let unknown = store.history(Some(&foreign), 10);
    assert_eq!(unknown.unwrap_err(), Error::NotFound);
}

#[test]
fn the_file_reveals_nothing_without_its_key() {
    let scratch = Scratch::new("key");
    lisbon(&scratch.db());

    let wrong = Sqlite::open(&scratch.db(), &[8; 32]);
    assert!(matches!(wrong, Err(Error::Storage(_))));
    let mut bytes = std::fs::read(scratch.db()).unwrap();
    if let Ok(wal) = std::fs::read(scratch.0.join("replica.db-wal")) {
        bytes.extend(wal);
    }
    assert!(!bytes.starts_with(b"SQLite format 3"));
    for plain in [&b"Lisbon"[..], b"Trips", b"benni"] {
        assert!(!bytes.windows(plain.len()).any(|window| window == plain));
    }
}

#[test]
fn a_database_that_holds_a_workspace_takes_no_second_one() {
    let scratch = Scratch::new("second");
    lisbon(&scratch.db());
    let store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    let second = Workspace::create(PersonId::new("mallory"), store);
    assert!(matches!(second, Err(Error::Storage(_))));
}

#[test]
fn a_long_edited_document_loads_from_a_snapshot() {
    let scratch = Scratch::new("compact");
    let (page, block) = lisbon(&scratch.db());
    let mut workspace = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();
    for at in 0..1_200 {
        let edit = Mutation::EditText {
            page: page.clone(),
            block: block.clone(),
            at,
            delete: 0,
            insert: "x".into(),
        };
        apply(&mut workspace, edit);
    }
    drop(workspace);

    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    let chunks = store.chunks(&Document::Page(page.clone())).unwrap();
    assert!(chunks.len() < 1_000, "{} chunks", chunks.len());
    let mut workspace = Workspace::open(store).unwrap();
    let text = workspace.text(&page, &block).unwrap().unwrap();
    assert_eq!(text, format!("{}Five days in Lisbon", "x".repeat(1_200)));
}

#[test]
fn one_session_at_a_time_holds_the_replica() {
    let scratch = Scratch::new("exclusive");
    lisbon(&scratch.db());
    let first = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();

    let Err(Error::Storage(second)) = Sqlite::open(&scratch.db(), &KEY) else {
        panic!("a second session opened the replica")
    };
    assert_eq!(second, "another Liquid session holds this replica");
    drop(first);
    assert!(Sqlite::open(&scratch.db(), &KEY).is_ok());
}

#[test]
fn history_keeps_a_number_cell_exactly() {
    let scratch = Scratch::new("number");
    let (page, _) = lisbon(&scratch.db());
    let mut workspace = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();
    let add = Mutation::AddDatabaseBlock {
        page: page.clone(),
        after: None,
        name: "Expenses".into(),
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add) else {
        panic!("a Database")
    };
    let field = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let amount = Mutation::AddField {
        page: page.clone(),
        block: block.clone(),
        place: Place::After(field),
        name: "Amount".into(),
        kind: liquid_core::FieldKind::Number,
    };
    let Some(Created::Field(amount)) = apply(&mut workspace, amount) else {
        panic!("a field")
    };
    let row = Mutation::AddRow {
        page: page.clone(),
        block: block.clone(),
        place: Place::Last,
    };
    let Some(Created::Row(row)) = apply(&mut workspace, row) else {
        panic!("a row")
    };
    // A value serde_json reads back one bit off without its exact float parsing.
    let value = 51.248178375505404;
    let set = Mutation::SetCell {
        page,
        block,
        row,
        field: amount,
        value: liquid_core::Cell::Number(value),
    };
    apply(&mut workspace, set.clone());
    drop(workspace);

    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    let newest = store.history(None, 1).unwrap().remove(0);
    assert_eq!(newest.operation, Operation::Mutation(set));
}
