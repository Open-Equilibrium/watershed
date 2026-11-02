//! The pipeline's invariant across documents: no write bypasses it.

use std::collections::HashSet;

use automerge::AutoCommit;

use crate::history::Header;
use crate::{Actor, Created, Document, Mutation, Origin, PersonId, Request, Workspace};

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
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let apply = |workspace: &mut Workspace, mutation| {
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

    let recorded: HashSet<_> = workspace
        .history()
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
    let documents = std::iter::once((Document::Workspace, &mut workspace.workspace))
        .chain(
            workspace
                .areas
                .iter_mut()
                .map(|(id, doc)| (Document::Area(id.clone()), doc)),
        )
        .chain(
            workspace
                .pages
                .iter_mut()
                .map(|(id, doc)| (Document::Page(id.clone()), doc)),
        );
    for (document, doc) in documents {
        for (hash, header) in changes(doc) {
            let header = header.expect("every change names its Action");
            stored.insert((
                document.clone(),
                hash,
                serde_json::to_string(&header).unwrap(),
            ));
        }
    }
    assert_eq!(stored, recorded);
    assert_eq!(workspace.history().len(), 6);
}
