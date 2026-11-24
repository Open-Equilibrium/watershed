#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    AreaId, BlockId, Created, Document, Error, Found, Hit, Layout, MemoryStore, Mutation, PageId,
    Store, ViewId,
};

struct Home {
    workspace: Workspace,
    area: AreaId,
}

impl Home {
    fn new() -> Self {
        let mut workspace = workspace();
        let Created::Area(area) = created(
            &mut workspace,
            Mutation::CreateArea {
                name: "Private".into(),
            },
        ) else {
            panic!("an area")
        };
        Self { workspace, area }
    }

    fn page(&mut self, title: &str) -> PageId {
        let Created::Page(page) = created(
            &mut self.workspace,
            Mutation::CreatePage {
                area: self.area.clone(),
                parent: None,
                title: title.into(),
            },
        ) else {
            panic!("a Page")
        };
        page
    }

    fn text(&mut self, page: &PageId, text: &str) -> (BlockId, ViewId) {
        let Created::Block { block, view } = created(
            &mut self.workspace,
            Mutation::AddTextBlock {
                page: page.clone(),
                after: None,
                text: text.into(),
            },
        ) else {
            panic!("a Text Block")
        };
        (block, view)
    }

    fn edit(&mut self, mutation: Mutation) -> Result<(), Error> {
        apply(&mut self.workspace, mutation).map(|_| ())
    }

    fn found(&mut self, query: &str, found: Found) -> Vec<Hit> {
        self.workspace.search(query, found, 10).unwrap()
    }

    fn ids(&mut self, query: &str, found: Found) -> Vec<String> {
        self.found(query, found)
            .into_iter()
            .map(|hit| hit.id)
            .collect()
    }
}

#[test]
fn titles_names_and_text_are_found_by_the_start_of_their_words() {
    let mut home = Home::new();
    let trip = home.page("Trip to Lisbon");
    let budget = home.page("Household budget");
    let (block, _) = home.text(&trip, "We expect rain on Tuesday.");
    let Created::Block { block: table, .. } = created(
        &mut home.workspace,
        Mutation::AddDatabaseBlock {
            page: budget,
            after: None,
            name: "Expenses".into(),
            layout: Layout::Table,
        },
    ) else {
        panic!("a Table")
    };
    assert_eq!(home.ids("lis", Found::Page), [trip.to_string()]);
    assert_eq!(home.ids("exp", Found::Table), [table.to_string()]);
    let text = home.found("exp", Found::Text);
    assert_eq!(text.len(), 1);
    assert_eq!(
        (text[0].id.as_str(), text[0].page.as_ref()),
        (block.as_str(), Some(&trip))
    );
    assert!(text[0].text.contains("expect"));
    assert!(
        home.ids("pect", Found::Text).is_empty(),
        "words match from their start"
    );
    assert_eq!(home.ids("RAIN tues", Found::Text), [block.to_string()]);
    assert!(home.ids("rain monday", Found::Text).is_empty());
    assert!(home.ids("  ", Found::Page).is_empty());
    assert!(home.ids("\"", Found::Page).is_empty());
}

#[test]
fn edits_renames_and_removals_keep_the_index_current() {
    let mut home = Home::new();
    let trip = home.page("Trip to Lisbon");
    let (block, view) = home.text(&trip, "Book the flat");
    home.edit(Mutation::EditText {
        page: trip.clone(),
        block: block.clone(),
        at: 13,
        delete: 0,
        insert: " in Alfama".into(),
    })
    .unwrap();
    assert_eq!(home.ids("alfama", Found::Text), [block.to_string()]);
    home.edit(Mutation::RenamePage {
        page: trip.clone(),
        title: "Trip to Porto".into(),
    })
    .unwrap();
    assert!(home.ids("lisbon", Found::Page).is_empty());
    assert_eq!(home.ids("porto", Found::Page), [trip.to_string()]);
    home.edit(Mutation::RemoveView { page: trip, view })
        .unwrap();
    assert!(home.ids("alfama", Found::Text).is_empty());
}

#[test]
fn a_workspace_held_as_a_version_finds_its_pages_once_opened() {
    let mut home = Home::new();
    let trip = home.page("Trip to Lisbon");
    let documents = [
        Document::Workspace,
        Document::Area(home.area.clone()),
        Document::Page(trip.clone()),
    ];
    let mut store = home.workspace.into_store();
    let held = documents.map(|document| {
        let bytes = store.chunks(&document).unwrap().concat();
        (document, bytes)
    });
    let mut opened = Workspace::open(MemoryStore::holding(held.to_vec())).unwrap();
    assert_eq!(
        opened.search("lisbon", Found::Page, 10).unwrap()[0].id,
        trip.to_string()
    );
}
