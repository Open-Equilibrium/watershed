//! The pipeline's invariant across documents: no write bypasses it.

use std::collections::HashSet;

use automerge::AutoCommit;

use crate::history::Header;
use crate::{
    Action, ActionId, Actor, Commit, Created, Document, Error, MemoryStore, Mutation, Origin,
    PersonId, Request, Store, Workspace,
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
    // The store, not the documents in memory, is what another session loads.
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

/// A store whose commits fail while `failing` is set.
#[derive(Default)]
struct Flaky {
    inner: MemoryStore,
    failing: bool,
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
