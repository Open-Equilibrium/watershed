#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use liquid_core::{
    Actor, Created, Document, Error, Found, Mutation, Operation, Origin, PageId, PersonId, Place,
    Request, Store, Workspace,
};
use liquid_storage::Sqlite;

const KEY: [u8; 32] = [7; 32];

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

    fn on_disk(&self) -> Vec<u8> {
        let mut bytes = std::fs::read(self.db()).unwrap();
        bytes.extend(std::fs::read(self.0.join("replica.db-wal")).unwrap_or_default());
        bytes
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

fn lisbon(path: &Path) -> (Workspace<Sqlite>, PageId, liquid_core::BlockId) {
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
    (workspace, page, block)
}

#[test]
fn an_acknowledged_write_is_there_after_a_reopen() {
    let scratch = Scratch::new("reopen");
    let (_, page, block) = lisbon(&scratch.db());

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
    let (workspace, _, _) = lisbon(&scratch.db());
    let wal = std::fs::read(scratch.0.join("replica.db-wal")).unwrap();
    assert!(!wal.is_empty(), "the writes are in the log");
    let bytes = scratch.on_disk();
    assert!(!bytes.starts_with(b"SQLite format 3"));
    for plain in [&b"Lisbon"[..], b"Trips", b"benni"] {
        assert!(!bytes.windows(plain.len()).any(|window| window == plain));
    }
    drop(workspace);
    let wrong = Sqlite::open(&scratch.db(), &[8; 32]);
    assert!(matches!(wrong, Err(Error::Storage(_))));
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
    let (_, page, block) = lisbon(&scratch.db());
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
    let (_, page, _) = lisbon(&scratch.db());
    let mut workspace = Workspace::open(Sqlite::open(&scratch.db(), &KEY).unwrap()).unwrap();
    let add = Mutation::AddDatabaseBlock {
        page: page.clone(),
        after: None,
        name: "Expenses".into(),
        layout: liquid_core::Layout::Table,
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add) else {
        panic!("a Database")
    };
    let field = workspace.database(&block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let amount = Mutation::AddField {
        block: block.clone(),
        place: Place::After(field),
        name: "Amount".into(),
        kind: liquid_core::FieldKind::Number,
    };
    let Some(Created::Field(amount)) = apply(&mut workspace, amount) else {
        panic!("a field")
    };
    let row = Mutation::AddRow {
        block: block.clone(),
        place: Place::Last,
    };
    let Some(Created::Row(row)) = apply(&mut workspace, row) else {
        panic!("a row")
    };
    let value = 51.248178375505404;
    let set = Mutation::SetCell {
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

#[test]
fn an_attachment_is_kept_encrypted_and_there_after_a_reopen() {
    let scratch = Scratch::new("attachment");
    let (mut workspace, ..) = lisbon(&scratch.db());
    let file = b"Tram 28 timetable ".repeat(100_000)[..liquid_core::PART + 1000].to_vec();
    let mut upload = workspace.begin_upload(&owner(), file.len() as u64).unwrap();
    workspace.upload(&mut upload, &file).unwrap();
    let attachment = workspace.finish_upload(upload).unwrap();

    let plain = b"Tram 28 timetable";
    let on_disk = scratch.on_disk();
    assert!(!on_disk.windows(plain.len()).any(|window| window == plain));

    drop(workspace);
    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    assert_eq!(
        store.attachment_size(&attachment.id).unwrap(),
        Some(file.len() as u64)
    );
    let mut read = store.part(&attachment.id, 0).unwrap().unwrap();
    read.extend(store.part(&attachment.id, 1).unwrap().unwrap());
    assert_eq!(read, file);
    assert_eq!(store.part(&attachment.id, 2).unwrap(), None);
}

#[test]
fn a_device_value_is_there_after_a_reopen_and_the_oldest_makes_room() {
    let scratch = Scratch::new("device");
    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    store.remember("tab/v", Some("t"), 2).unwrap();
    store.remember("gone", Some("x"), 2).unwrap();
    store.remember("gone", None, 2).unwrap();
    drop(store);
    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    assert_eq!(store.remembered("tab/v").unwrap().as_deref(), Some("t"));
    assert_eq!(store.remembered("gone").unwrap(), None);
    store.remember("later", Some("u"), 2).unwrap();
    store.remember("tab/v", Some("t2"), 2).unwrap();
    store.remember("newest", Some("n"), 2).unwrap();
    assert_eq!(store.remembered("later").unwrap(), None);
    assert_eq!(store.remembered("tab/v").unwrap().as_deref(), Some("t2"));
    assert_eq!(store.remembered("newest").unwrap().as_deref(), Some("n"));
    store.remember("none", Some("x"), 0).unwrap();
    for key in ["none", "tab/v", "newest"] {
        assert_eq!(
            store.remembered(key).unwrap(),
            None,
            "no room keeps nothing"
        );
    }
}

#[test]
fn the_search_index_is_kept_in_the_encrypted_replica() {
    let scratch = Scratch::new("search");
    let (mut workspace, page, block) = lisbon(&scratch.db());
    apply(
        &mut workspace,
        Mutation::EditText {
            page: page.clone(),
            block: block.clone(),
            at: 19,
            delete: 0,
            insert: ", then Porto \"by train\"".into(),
        },
    );
    let title = "The long weekend we spent walking up and down the seven hills of Lisbon";
    apply(
        &mut workspace,
        Mutation::RenamePage {
            page,
            title: title.into(),
        },
    );
    let mut store = workspace.into_store();
    store.remember("recent", Some("p"), 1).unwrap();
    store.remember("index-version", None, 1).unwrap();
    assert_eq!(
        store.index_version().unwrap(),
        liquid_core::INDEX_VERSION,
        "device values never evict the index version"
    );
    drop(store);
    let store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    let mut workspace = Workspace::open(store).unwrap();
    let hits = workspace.search("port tra", Found::Text, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, block.to_string());
    assert!(hits[0].text.contains("Porto"));
    let pages = workspace.search("lisb", Found::Page, 10).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].text, title, "a title is found whole");
    assert!(workspace.search("\"", Found::Text, 10).unwrap().is_empty());
    drop(workspace);
    assert!(
        !scratch.on_disk().windows(5).any(|bytes| bytes == b"Porto"),
        "the index is encrypted with the rest"
    );
}

fn keyed(path: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(path).unwrap();
    let hex: String = KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .unwrap();
    connection
}

#[cfg(target_vendor = "apple")]
#[test]
fn on_apple_the_replica_is_encrypted_by_the_systems_crypto() {
    let scratch = Scratch::new("provider");
    drop(lisbon(&scratch.db()));
    let provider: String = keyed(&scratch.db())
        .query_row("PRAGMA cipher_provider", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provider, "commoncrypto");
}

#[test]
fn a_replica_written_by_the_openssl_build_still_opens() {
    let scratch = Scratch::new("openssl");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openssl-written.db"),
        scratch.db(),
    )
    .unwrap();
    let connection = keyed(&scratch.db());
    let text: String = connection
        .query_row("SELECT v FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(text, "written by the OpenSSL build");
    let mut check = connection.prepare("PRAGMA cipher_integrity_check").unwrap();
    let problems: Vec<String> = check
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_page_s_earlier_versions_deleted_for_good_stay_gone_after_a_reopen() {
    let scratch = Scratch::new("purge");
    let (mut workspace, page, block) = lisbon(&scratch.db());
    let secret = apply(
        &mut workspace,
        Mutation::EditText {
            page: page.clone(),
            block: block.clone(),
            at: 0,
            delete: 0,
            insert: "hunter2 ".into(),
        },
    );
    assert!(secret.is_none());
    let typed = workspace.history(None, 1).unwrap().remove(0).id;
    apply(
        &mut workspace,
        Mutation::EditText {
            page: page.clone(),
            block: block.clone(),
            at: 0,
            delete: 8,
            insert: String::new(),
        },
    );
    workspace
        .purge(&owner(), &liquid_core::Purge::Page(page.clone()))
        .unwrap();
    let wal = std::fs::metadata(scratch.0.join("replica.db-wal")).map_or(0, |file| file.len());
    assert_eq!(wal, 0);
    drop(workspace);

    let mut store = Sqlite::open(&scratch.db(), &KEY).unwrap();
    let touching = store
        .history_of(&[Document::Page(page.clone())], None, 100)
        .unwrap();
    assert!(matches!(touching[0].operation, Operation::Purge { .. }));
    let mut workspace = Workspace::open(store).unwrap();
    assert_eq!(
        workspace.text(&page, &block).unwrap().unwrap(),
        "Five days in Lisbon"
    );
    let records = serde_json::to_string(&workspace.history(None, 100).unwrap()).unwrap();
    assert!(!records.contains("hunter2"));
    assert_eq!(
        workspace.revert(&owner(), &[typed]).unwrap_err(),
        Error::Purged
    );
    drop(workspace);
    let indexed: i64 = keyed(&scratch.db())
        .query_row(
            "SELECT count(*) FROM search_data WHERE instr(block, CAST('hunter2' AS BLOB)) > 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0);
}

#[test]
fn a_version_before_an_undo_restores_from_the_disk() {
    let scratch = Scratch::new("restore");
    let (mut workspace, page, block) = lisbon(&scratch.db());
    apply(
        &mut workspace,
        Mutation::EditText {
            page: page.clone(),
            block: block.clone(),
            at: 0,
            delete: 0,
            insert: "Ours: ".into(),
        },
    );
    let typed = workspace.history(None, 1).unwrap().remove(0).id;
    workspace.undo(&owner()).unwrap().unwrap();
    workspace.redo(&owner()).unwrap().unwrap();
    workspace
        .revert(&owner(), std::slice::from_ref(&typed))
        .unwrap();
    workspace.close_page(&page);
    workspace.restore(&owner(), &page, &typed).unwrap();
    assert_eq!(
        workspace.text(&page, &block).unwrap().unwrap(),
        "Ours: Five days in Lisbon"
    );
}
