use std::collections::HashSet;

use automerge::AutoCommit;

use crate::history::Header;
use crate::{
    Action, ActionId, Actor, AttachmentId, Commit, Created, Document, Error, MemoryStore, Mutation,
    Origin, PersonId, Request, Store, Workspace,
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

#[test]
fn every_change_of_every_document_belongs_to_a_recorded_action() {
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
            .created
    };
    let Some(Created::Area(area)) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    ) else {
        panic!("an area")
    };
    let create_page = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    let Some(Created::Page(page)) = apply(&mut workspace, create_page) else {
        panic!("a Page")
    };
    let add = Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days".into(),
    };
    let Some(Created::Block { block, view }) = apply(&mut workspace, add) else {
        panic!("a Block")
    };
    let edit = Mutation::EditText {
        page: page.clone(),
        block,
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    };
    apply(&mut workspace, edit);
    apply(&mut workspace, Mutation::RemoveView { page, view });

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

#[derive(Default)]
struct Flaky {
    inner: MemoryStore,
    failing: bool,
    index_failing: bool,
    history_failing: bool,
}

impl Store for Flaky {
    fn chunks(&mut self, document: &Document) -> Result<Vec<Vec<u8>>, Error> {
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
fn a_write_the_store_refuses_is_not_acknowledged_and_leaves_no_trace() {
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let apply = |workspace: &mut Workspace<Flaky>, mutation| {
        workspace.apply(Request {
            actor: owner.clone(),
            origin: Origin::Ui,
            mutation,
        })
    };
    let Ok(Some(Created::Area(area))) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    )
    .map(|applied| applied.created) else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area: area.clone(),
        parent: None,
        title: "Lisbon".into(),
    };
    let Ok(Some(Created::Page(page))) =
        apply(&mut workspace, create).map(|applied| applied.created)
    else {
        panic!("a Page")
    };
    let add = Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days".into(),
    };
    let Ok(Some(Created::Block { block, .. })) =
        apply(&mut workspace, add).map(|applied| applied.created)
    else {
        panic!("a Block")
    };
    let recorded = workspace.history(None, usize::MAX).unwrap().len();

    workspace.store.failing = true;
    let edit = Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    };
    let rename = Mutation::RenamePage {
        page: page.clone(),
        title: "Porto".into(),
    };
    let new_page = Mutation::CreatePage {
        area: area.clone(),
        parent: None,
        title: "Porto".into(),
    };
    for mutation in [edit.clone(), rename, new_page] {
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
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
            .created
    };
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
    let mut views: Vec<crate::ViewId> = Vec::new();
    for round in 0..60usize {
        let count = workspace.view_count(&page).unwrap();
        if round % 5 == 4 && count > 0 {
            let view = workspace
                .views(&page, round % count, 1)
                .unwrap()
                .remove(0)
                .id;
            let remove = Mutation::RemoveView {
                page: page.clone(),
                view,
            };
            apply(&mut workspace, remove);
            continue;
        }
        let after =
            (round % 3 > 0 && !views.is_empty()).then(|| views[round % views.len()].clone());
        let add = Mutation::AddTextBlock {
            page: page.clone(),
            after,
            text: format!("Block {round}"),
        };
        if let Some(Created::Block { view, .. }) = apply(&mut workspace, add) {
            views.push(view);
        }
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
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
            .created
    };
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
        text: "Five days".into(),
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add) else {
        panic!("a Block")
    };

    workspace.close_page(&page);
    assert!(!workspace.docs.contains_key(&Document::Page(page.clone())));
    assert_eq!(workspace.view_count(&page).unwrap(), 1);
    assert_eq!(workspace.text(&page, &block).unwrap().unwrap(), "Five days");
}

#[test]
fn a_block_of_an_unknown_kind_is_neither_read_nor_changed_as_text() {
    use automerge::transaction::Transactable;

    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace.apply(Request {
            actor: owner.clone(),
            origin: Origin::Ui,
            mutation,
        })
    };
    let Ok(Some(Created::Area(area))) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    )
    .map(|applied| applied.created) else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    let Ok(Some(Created::Page(page))) = apply(&mut workspace, create).map(|a| a.created) else {
        panic!("a Page")
    };
    let add = Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days".into(),
    };
    let Ok(Some(Created::Block { block, view })) = apply(&mut workspace, add).map(|a| a.created)
    else {
        panic!("a Text Block")
    };
    let below = Mutation::AddTextBlock {
        page: page.clone(),
        after: Some(view.clone()),
        text: "Porto".into(),
    };
    let Ok(Some(Created::Block { view: below, .. })) =
        apply(&mut workspace, below).map(|a| a.created)
    else {
        panic!("a Text Block")
    };
    let loaded = workspace
        .docs
        .get_mut(&Document::Page(page.clone()))
        .unwrap();
    let entry = crate::model::live(&loaded.doc, crate::model::BLOCKS, block.as_str()).unwrap();
    loaded.doc.put(&entry, crate::model::KIND, "chart").unwrap();
    loaded.doc.commit();

    assert_eq!(workspace.text(&page, &block), Ok(None));
    let edit = Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "x".into(),
    };
    let split = Mutation::SplitTextBlock {
        page: page.clone(),
        block,
        view,
        at: 0,
        style: crate::TextStyle::Paragraph,
    };
    let join = Mutation::JoinTextBlock { page, view: below };
    for refused in [edit, split, join] {
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
    use crate::{BlockId, ViewId};

    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
            .created
    };
    let Some(Created::Area(area)) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    ) else {
        panic!("an area")
    };
    let Some(Created::Page(page)) = apply(
        &mut workspace,
        Mutation::CreatePage {
            area,
            parent: None,
            title: "Lisbon".into(),
        },
    ) else {
        panic!("a Page")
    };
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
        style: crate::TextStyle::Paragraph,
    };
    let Some(Created::Block { view: added, .. }) = apply(&mut workspace, split) else {
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
    let apply = |workspace: &mut Workspace<MemoryStore>, mutation| {
        workspace
            .apply(Request {
                actor: Actor::Person(owner.clone()),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
    };
    let Some(Created::Area(area)) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Private".into(),
        },
    )
    .created
    else {
        panic!("an area")
    };
    let Some(Created::Table(table)) = apply(
        &mut workspace,
        Mutation::CreateTable {
            area,
            name: "Bills".into(),
        },
    )
    .created
    else {
        panic!("a Table")
    };
    let Some(Created::Field(category)) = apply(
        &mut workspace,
        Mutation::AddField {
            block: table.clone(),
            place: crate::Place::Last,
            name: "Category".into(),
            kind: crate::FieldKind::Select,
        },
    )
    .created
    else {
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
    let Some(Created::Automation(automation)) = apply(
        &mut workspace,
        Mutation::CreateAutomation {
            setup: Box::new(setup.clone()),
        },
    )
    .created
    else {
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
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let mut apply = |mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
            .created
    };
    let Some(Created::Area(area)) = apply(Mutation::CreateArea {
        name: "Trips".into(),
    }) else {
        panic!("an area")
    };
    let Some(Created::Page(page)) = apply(Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    }) else {
        panic!("a Page")
    };
    let Some(Created::Block { block, .. }) = apply(Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days".into(),
    }) else {
        panic!("a Block")
    };
    apply(Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    });

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
    let owner = Actor::Person(PersonId::new("benni"));
    let mut workspace = Workspace::create(PersonId::new("benni"), Flaky::default()).unwrap();
    let mut apply = |mutation| {
        workspace
            .apply(Request {
                actor: owner.clone(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
    };
    let Some(Created::Area(area)) = apply(Mutation::CreateArea {
        name: "Trips".into(),
    })
    .created
    else {
        panic!("an area")
    };
    let Some(Created::Page(page)) = apply(Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    })
    .created
    else {
        panic!("a Page")
    };
    let Some(Created::Block { block, .. }) = apply(Mutation::AddTextBlock {
        page: page.clone(),
        after: None,
        text: "Five days".into(),
    })
    .created
    else {
        panic!("a Block")
    };
    let typed = apply(Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "Our ".into(),
    })
    .action;

    workspace.store.history_failing = true;
    let owner = Actor::Person(PersonId::new("benni"));
    assert!(workspace.purge(&owner, &crate::Purge::Page(page.clone())).is_err());
    workspace.store.history_failing = false;
    workspace.revert(&owner, &[typed]).unwrap();
    assert_eq!(workspace.text(&page, &block).unwrap().unwrap(), "Five days");
}
