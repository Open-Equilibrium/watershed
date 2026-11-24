use std::collections::HashSet;

use automerge::AutoCommit;

use crate::history::Header;
use crate::{
    Action, ActionId, Actor, Applied, AreaId, AttachmentId, BlockId, Commit, Created, Document,
    Error, Found, MemoryStore, Mutation, Origin, PageId, PersonId, Request, Store, TextStyle,
    ViewId, Workspace,
};

fn changes(doc: &mut AutoCommit) -> Vec<(automerge::ChangeHash, Option<Header>)> {
    doc.get_changes(&[])
        .iter()
        .map(|change| {
            let header = change
                .message()
                .and_then(|message| serde_json::from_str(message).ok());
            (change.hash(), header)
        })
        .collect()
}

fn apply<S: Store>(workspace: &mut Workspace<S>, mutation: Mutation) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

fn created<S: Store>(workspace: &mut Workspace<S>, mutation: Mutation) -> Created {
    apply(workspace, mutation).unwrap().created.unwrap()
}

fn new_page<S: Store>(workspace: &mut Workspace<S>) -> (AreaId, PageId) {
    let Created::Area(area) = created(
        workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    ) else {
        panic!("an area")
    };
    let Created::Page(page) = created(
        workspace,
        Mutation::CreatePage {
            area: area.clone(),
            parent: None,
            title: "Lisbon".into(),
        },
    ) else {
        panic!("a Page")
    };
    (area, page)
}

fn add_text<S: Store>(
    workspace: &mut Workspace<S>,
    page: &PageId,
    after: Option<ViewId>,
    text: &str,
) -> (BlockId, ViewId) {
    let Created::Block { block, view } = created(
        workspace,
        Mutation::AddTextBlock {
            page: page.clone(),
            after,
            text: text.into(),
        },
    ) else {
        panic!("a Text Block")
    };
    (block, view)
}

fn edit_text(page: &PageId, block: &BlockId) -> Mutation {
    Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    }
}

#[derive(Default)]
struct Flaky {
    inner: MemoryStore,
    failing: bool,
    index_failing: bool,
    history_failing: bool,
    unreadable: Option<Document>,
}

impl Store for Flaky {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error> {
        if self.unreadable.as_ref() == Some(document) {
            return Err(Error::Storage("unreadable".into()));
        }
        self.inner.chunks(document)
    }

    fn commit(&mut self, commit: &Commit<'_>) -> Result<(), Error> {
        if self.failing {
            return Err(Error::Storage("disk full".into()));
        }
        self.inner.commit(commit)
    }

    fn history(&mut self, before: Option<&ActionId>, limit: usize) -> Result<Vec<Action>, Error> {
        self.inner.history(before, limit)
    }

    fn history_of(
        &mut self,
        documents: &[Document],
        before: Option<&ActionId>,
        limit: usize,
    ) -> Result<Vec<Action>, Error> {
        if self.history_failing {
            return Err(Error::Storage("disk full".into()));
        }
        self.inner.history_of(documents, before, limit)
    }

    fn put_part(&mut self, upload: &str, index: u64, bytes: &[u8]) -> Result<(), Error> {
        if self.failing {
            return Err(Error::Storage("disk full".into()));
        }
        self.inner.put_part(upload, index, bytes)
    }

    fn finish_upload(
        &mut self,
        upload: &str,
        attachment: &AttachmentId,
        size: u64,
        parts: u64,
    ) -> Result<(), Error> {
        self.inner.finish_upload(upload, attachment, size, parts)
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), Error> {
        self.inner.discard_upload(upload)
    }

    fn attachment_size(&mut self, attachment: &AttachmentId) -> Result<Option<u64>, Error> {
        self.inner.attachment_size(attachment)
    }

    fn part(&mut self, attachment: &AttachmentId, index: u64) -> Result<Option<Vec<u8>>, Error> {
        self.inner.part(attachment, index)
    }

    fn remembered(&mut self, key: &str) -> Result<Option<String>, Error> {
        self.inner.remembered(key)
    }

    fn remember(&mut self, key: &str, value: Option<&str>, most: usize) -> Result<(), Error> {
        self.inner.remember(key, value, most)
    }

    fn source_path(&mut self, source: &crate::SourceId) -> Result<Option<String>, Error> {
        self.inner.source_path(source)
    }

    fn locate(&mut self, source: &crate::SourceId, path: Option<&str>) -> Result<(), Error> {
        self.inner.locate(source, path)
    }

    fn index(&mut self, writes: &[crate::IndexWrite], version: Option<u32>) -> Result<(), Error> {
        if self.index_failing {
            return Err(Error::Storage("disk full".into()));
        }
        self.inner.index(writes, version)
    }

    fn index_version(&mut self) -> Result<u32, Error> {
        self.inner.index_version()
    }

    fn search(
        &mut self,
        words: &[String],
        found: crate::Found,
        limit: usize,
    ) -> Result<Vec<crate::Hit>, Error> {
        self.inner.search(words, found, limit)
    }
}

#[test]
fn every_change_of_every_document_belongs_to_a_recorded_action() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let (block, view) = add_text(&mut workspace, &page, None, "Five days");
    apply(&mut workspace, edit_text(&page, &block)).unwrap();
    apply(&mut workspace, Mutation::RemoveView { page, view }).unwrap();

    let history = workspace.history(None, usize::MAX).unwrap();
    let recorded: HashSet<_> = history
        .iter()
        .flat_map(|action| {
            let header = Header::of(action);
            action.changes.iter().map(move |(document, hash)| {
                (
                    document.clone(),
                    *hash,
                    serde_json::to_string(&header).unwrap(),
                )
            })
        })
        .collect();
    let mut stored = HashSet::new();
    let documents: Vec<Document> = workspace.docs.keys().cloned().collect();
    assert_eq!(documents.len(), 3, "the Workspace, one area and one Page");
    for document in documents {
        let chunks = workspace.store.chunks(&document).unwrap();
        let mut doc = AutoCommit::load(&chunks.concat()).unwrap();
        for (hash, header) in changes(&mut doc) {
            let header = header.expect("every change names its Action");
            stored.insert((
                document.clone(),
                hash,
                serde_json::to_string(&header).unwrap(),
            ));
        }
    }
    assert_eq!(stored, recorded);
    assert_eq!(history.len(), 6);
}

#[test]
fn a_write_the_store_refuses_is_not_acknowledged_and_leaves_no_trace() {
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let (area, page) = new_page(&mut workspace);
    let (block, _) = add_text(&mut workspace, &page, None, "Five days");
    let recorded = workspace.history(None, usize::MAX).unwrap().len();

    workspace.store.failing = true;
    let edit = edit_text(&page, &block);
    let rename = Mutation::RenamePage {
        page: page.clone(),
        title: "Porto".into(),
    };
    let second_page = Mutation::CreatePage {
        area: area.clone(),
        parent: None,
        title: "Porto".into(),
    };
    for mutation in [edit.clone(), rename, second_page] {
        let refused = apply(&mut workspace, mutation);
        assert_eq!(refused.unwrap_err(), Error::Storage("disk full".into()));
    }
    assert_eq!(workspace.text(&page, &block).unwrap().unwrap(), "Five days");
    let titles: Vec<_> = workspace
        .pages(&area)
        .unwrap()
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(titles, ["Lisbon"]);
    assert_eq!(workspace.history(None, usize::MAX).unwrap().len(), recorded);

    workspace.store.failing = false;
    apply(&mut workspace, edit).unwrap();
    assert_eq!(
        workspace.text(&page, &block).unwrap().unwrap(),
        "Our Five days"
    );
}

#[test]
fn the_view_order_index_matches_the_page_document() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let mut views: Vec<ViewId> = Vec::new();
    for round in 0..60usize {
        let count = workspace.view_count(&page).unwrap();
        if round % 5 == 4 && count > 0 {
            let view = workspace
                .views(&page, round % count, 1)
                .unwrap()
                .remove(0)
                .id;
            apply(
                &mut workspace,
                Mutation::RemoveView {
                    page: page.clone(),
                    view,
                },
            )
            .unwrap();
            continue;
        }
        let after =
            (round % 3 > 0 && !views.is_empty()).then(|| views[round % views.len()].clone());
        let (_, view) = add_text(&mut workspace, &page, after, &format!("Block {round}"));
        views.push(view);
        views.retain(|view| {
            workspace
                .views(&page, 0, usize::MAX)
                .unwrap()
                .iter()
                .any(|shown| &shown.id == view)
        });
    }

    let kept = workspace.views(&page, 0, usize::MAX).unwrap();
    let loaded = &workspace.docs[&Document::Page(page)];
    let read = crate::order::Order::read(&loaded.doc).window(0, usize::MAX);
    assert_eq!(kept, read);
    assert!(kept.len() > 30);
}

#[test]
fn a_closed_page_leaves_memory_and_loads_again_unchanged() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let (block, _) = add_text(&mut workspace, &page, None, "Five days");

    workspace.close_page(&page);
    assert!(!workspace.docs.contains_key(&Document::Page(page.clone())));
    assert_eq!(workspace.view_count(&page).unwrap(), 1);
    assert_eq!(workspace.text(&page, &block).unwrap().unwrap(), "Five days");
}

#[test]
fn a_block_of_an_unknown_kind_is_neither_read_nor_changed_as_text() {
    use automerge::transaction::Transactable;

    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let (block, view) = add_text(&mut workspace, &page, None, "Five days");
    let (_, below) = add_text(&mut workspace, &page, Some(view.clone()), "Porto");
    let loaded = workspace
        .docs
        .get_mut(&Document::Page(page.clone()))
        .unwrap();
    let entry = crate::model::live(&loaded.doc, crate::model::BLOCKS, block.as_str()).unwrap();
    loaded.doc.put(&entry, crate::model::KIND, "chart").unwrap();
    loaded.doc.commit();

    assert_eq!(workspace.text(&page, &block), Ok(None));
    let split = Mutation::SplitTextBlock {
        page: page.clone(),
        block: block.clone(),
        view,
        at: 0,
        style: TextStyle::Paragraph,
    };
    let join = Mutation::JoinTextBlock {
        page: page.clone(),
        view: below,
    };
    for refused in [edit_text(&page, &block), split, join] {
        assert_eq!(
            apply(&mut workspace, refused).map(|_| ()),
            Err(Error::NotFound)
        );
    }
}

#[test]
fn a_start_split_puts_the_new_text_block_above_a_view_whose_key_another_shares() {
    use automerge::ObjType;
    use automerge::transaction::Transactable;

    use crate::model::{self, BLOCK, BLOCKS, KIND, SORT, STYLE, TEXT, VIEWS};

    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let loaded = workspace
        .docs
        .get_mut(&Document::Page(page.clone()))
        .unwrap();
    for (block, text) in [("b1", "Plan"), ("b2", "Day one")] {
        let fields = [(KIND, "text"), (STYLE, "paragraph")];
        let entry = model::insert(&mut loaded.doc, BLOCKS, block, &fields).unwrap();
        let content = loaded.doc.put_object(&entry, TEXT, ObjType::Text).unwrap();
        loaded.doc.splice_text(&content, 0, 0, text).unwrap();
        let view = format!("v-{block}");
        model::insert(
            &mut loaded.doc,
            VIEWS,
            &view,
            &[(BLOCK, block), (SORT, "a0")],
        )
        .unwrap();
    }
    loaded.doc.commit();

    let split = Mutation::SplitTextBlock {
        page: page.clone(),
        block: BlockId::stored("b2"),
        view: ViewId::stored("v-b2"),
        at: 0,
        style: TextStyle::Paragraph,
    };
    let Created::Block { view: added, .. } = created(&mut workspace, split) else {
        panic!("a Text Block")
    };
    let views = workspace.views(&page, 0, usize::MAX).unwrap();
    let at = |view: &ViewId| views.iter().position(|shown| &shown.id == view);
    assert!(at(&added) < at(&ViewId::stored("v-b2")), "{views:?}");
}

#[test]
fn an_upload_whose_part_the_store_refused_never_finishes() {
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let part = vec![7; crate::PART];
    let mut upload = workspace
        .begin_upload(&owner, 2 * crate::PART as u64)
        .unwrap();
    workspace.store.failing = true;
    assert!(workspace.upload(&mut upload, &part).is_err());
    workspace.store.failing = false;
    assert!(
        workspace.upload(&mut upload, &part).is_err(),
        "a failed upload takes no more"
    );
    assert!(workspace.finish_upload(upload).is_err());
}

#[test]
fn a_failed_index_version_write_leaves_a_created_workspace_usable() {
    let store = Flaky {
        index_failing: true,
        ..Flaky::default()
    };
    let workspace = Workspace::create(PersonId::new("benni"), store).unwrap();
    let mut store = workspace.store;
    store.index_failing = false;
    let mut reopened = Workspace::open(store).unwrap();
    assert_eq!(
        reopened.store.index_version().unwrap(),
        crate::INDEX_VERSION
    );
}

#[test]
fn a_run_whose_later_step_fails_leaves_no_row_of_an_earlier_one() {
    use automerge::transaction::Transactable;
    use automerge::{ROOT, ReadDoc};

    let owner = PersonId::new("benni");
    let mut workspace = Workspace::create(owner.clone(), MemoryStore::default()).unwrap();
    let Created::Area(area) = created(
        &mut workspace,
        Mutation::CreateArea {
            name: "Private".into(),
        },
    ) else {
        panic!("an area")
    };
    let Created::Table(table) = created(
        &mut workspace,
        Mutation::CreateTable {
            area,
            name: "Bills".into(),
        },
    ) else {
        panic!("a Table")
    };
    let Created::Field(category) = created(
        &mut workspace,
        Mutation::AddField {
            block: table.clone(),
            place: crate::Place::Last,
            name: "Category".into(),
            kind: crate::FieldKind::Select,
        },
    ) else {
        panic!("a field")
    };
    let setup = crate::AutomationSetup {
        name: "Copy".into(),
        trigger: Some(crate::Trigger::RowAdded(table.clone())),
        conditions: Vec::new(),
        steps: vec![crate::Step::AddRow {
            table: table.clone(),
            cells: Vec::new(),
        }],
    };
    let Created::Automation(automation) = created(
        &mut workspace,
        Mutation::CreateAutomation {
            setup: Box::new(setup.clone()),
        },
    ) else {
        panic!("an Automation")
    };
    let broken = crate::AutomationSetup {
        steps: vec![
            setup.steps[0].clone(),
            crate::Step::Set {
                field: category,
                value: crate::Cell::Select(crate::OptionId::stored(&"e".repeat(32))),
            },
        ],
        ..setup
    };
    let doc = workspace.doc(&Document::Workspace).unwrap();
    let map = match doc.get(ROOT, "automations").unwrap() {
        Some((_, map)) => map,
        None => panic!("the automations map"),
    };
    let entry = match doc.get(&map, automation.as_str()).unwrap() {
        Some((_, entry)) => entry,
        None => panic!("the Automation"),
    };
    doc.put(&entry, "setup", serde_json::to_string(&broken).unwrap())
        .unwrap();
    doc.commit();

    let ran = apply(
        &mut workspace,
        Mutation::AddRow {
            block: table.clone(),
            place: crate::Place::Last,
        },
    )
    .unwrap()
    .ran;
    assert_eq!(ran[0].outcome, Err(Error::NotFound));
    let rows = workspace
        .rows(
            &table,
            &crate::RowQuery {
                sort: None,
                filter: None,
                within: None,
                start: 0,
                limit: usize::MAX,
                search: None,
                totals: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(rows.count, 1);
}

#[test]
fn an_undo_the_store_refuses_leaves_the_text_as_it_was() {
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let (block, _) = add_text(&mut workspace, &page, None, "Five days");
    apply(&mut workspace, edit_text(&page, &block)).unwrap();

    workspace.store.failing = true;
    let owner = Actor::Person(PersonId::new("benni"));
    assert_eq!(
        workspace.undo(&owner).unwrap_err(),
        Error::Storage("disk full".into())
    );
    assert_eq!(
        workspace.text(&page, &block).unwrap().unwrap(),
        "Our Five days"
    );
}

#[test]
fn a_purge_that_fails_keeps_the_page_s_history() {
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let (block, _) = add_text(&mut workspace, &page, None, "Five days");
    let typed = apply(&mut workspace, edit_text(&page, &block))
        .unwrap()
        .action;

    workspace.store.history_failing = true;
    let owner = Actor::Person(PersonId::new("benni"));
    assert!(
        workspace
            .purge(&owner, &crate::Purge::Page(page.clone()))
            .is_err()
    );
    workspace.store.history_failing = false;
    workspace.revert(&owner, &[typed]).unwrap();
    assert_eq!(workspace.text(&page, &block).unwrap().unwrap(), "Five days");
}

#[test]
fn a_replica_without_its_index_builds_it_once_opened() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    add_text(&mut workspace, &page, None, "We fly on 12 November.");
    let mut store = workspace.into_store();
    store.drop_index();
    let mut workspace = Workspace::open(store).unwrap();
    assert_eq!(workspace.search("novem", Found::Text, 10).unwrap().len(), 1);
    assert_eq!(workspace.search("lisb", Found::Page, 10).unwrap().len(), 1);
    let long = "x".repeat(crate::MAX_FILTER_TEXT + 1);
    assert_eq!(
        workspace.search(&long, Found::Text, 10),
        Err(Error::TooLong)
    );
}

#[test]
fn an_unreadable_page_neither_stops_the_open_nor_the_rest_of_the_index() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (area, trip) = new_page(&mut workspace);
    add_text(&mut workspace, &trip, None, "We fly on 12 November.");
    let Created::Page(broken) = created(
        &mut workspace,
        Mutation::CreatePage {
            area,
            parent: None,
            title: "Broken".into(),
        },
    ) else {
        panic!("a Page")
    };
    add_text(&mut workspace, &broken, None, "Never read");
    let mut store = workspace.into_store();
    store.drop_index();
    store.spoil(&Document::Page(broken));
    let mut workspace = Workspace::open(store).unwrap();
    assert_eq!(workspace.search("novem", Found::Text, 10).unwrap().len(), 1);
    let mut store = workspace.into_store();
    assert_eq!(
        store.index_version().unwrap(),
        0,
        "the next open tries again"
    );
}

#[test]
fn an_unreadable_area_does_not_stop_the_open() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (area, _) = new_page(&mut workspace);
    let mut store = workspace.into_store();
    store.drop_index();
    store.spoil(&Document::Area(area));
    let workspace = Workspace::open(store).unwrap();
    let mut store = workspace.into_store();
    assert_eq!(
        store.index_version().unwrap(),
        0,
        "the next open tries again"
    );
}

#[test]
fn removing_a_table_view_whose_table_cannot_load_is_refused_whole() {
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let Created::Block { block, view } = created(
        &mut workspace,
        Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: None,
            name: "Bills".into(),
            layout: crate::Layout::Table,
        },
    ) else {
        panic!("a Table")
    };
    let mut store = workspace.into_store();
    store.unreadable = Some(Document::Table(block));
    let mut workspace = Workspace::open(store).unwrap();
    assert_eq!(
        apply(&mut workspace, Mutation::RemoveView { page, view }).map(|_| ()),
        Err(Error::Storage("unreadable".into()))
    );
}

fn saved<S: Store>(workspace: &mut Workspace<S>, document: &Document) -> Vec<u8> {
    workspace.docs.get_mut(document).unwrap().doc.save()
}

fn replica_of<S: Store>(workspace: &mut Workspace<S>, document: &Document, from: &[u8]) {
    workspace.docs.get_mut(document).unwrap().doc =
        crate::model::load(from, &automerge::ActorId::random()).unwrap();
}

fn merged(first: &[u8], second: &[u8]) -> AutoCommit {
    let mut merged = crate::model::load(first, &automerge::ActorId::random()).unwrap();
    let mut other = crate::model::load(second, &automerge::ActorId::random()).unwrap();
    merged.merge(&mut other).unwrap();
    merged
}

#[test]
fn two_replicas_that_each_make_a_table_keep_both() {
    use crate::model::{self, NAME, TABLES};

    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (area, _) = new_page(&mut workspace);
    let base = saved(&mut workspace, &Document::Workspace);
    let mut replicas = Vec::new();
    for name in ["Bills", "Trips"] {
        replica_of(&mut workspace, &Document::Workspace, &base);
        workspace.write_create_table(&area, name, None).unwrap();
        replicas.push(saved(&mut workspace, &Document::Workspace));
    }
    let both = merged(&replicas[0], &replicas[1]);
    let names: Vec<_> = model::sorted(&both, TABLES)
        .iter()
        .map(|entry| entry.get(NAME).unwrap_or_default().to_string())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn two_replicas_that_each_add_a_tab_to_a_duplicated_table_keep_both() {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let (_, page) = new_page(&mut workspace);
    let Created::Block { block, .. } = created(
        &mut workspace,
        Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: None,
            name: "Bills".into(),
            layout: crate::Layout::Table,
        },
    ) else {
        panic!("a Table")
    };
    let Created::Block { view, .. } = created(
        &mut workspace,
        Mutation::DuplicateTable {
            page: page.clone(),
            after: None,
            table: block,
            name: "Copy".into(),
            tabs: Vec::new(),
        },
    ) else {
        panic!("a Table")
    };
    let document = Document::Page(page.clone());
    let base = saved(&mut workspace, &document);
    let mut replicas = Vec::new();
    for name in ["Porto", "Lisbon"] {
        replica_of(&mut workspace, &document, &base);
        let add = Mutation::AddTab {
            page: page.clone(),
            view: view.clone(),
            after: None,
            setup: Box::new(crate::TabSetup {
                name: name.into(),
                ..crate::TabSetup::of(crate::Layout::Table)
            }),
        };
        apply(&mut workspace, add).unwrap();
        replicas.push(saved(&mut workspace, &document));
    }
    let both = merged(&replicas[0], &replicas[1]);
    let entry = crate::model::live(&both, crate::model::VIEWS, view.as_str()).unwrap();
    let names: Vec<_> = crate::tab::read(&both, &entry, view.as_str())
        .into_iter()
        .map(|tab| tab.setup.name)
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
}
