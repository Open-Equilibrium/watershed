#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, AreaId, BlockId, Created, Document, Error, Found, Hit, MemoryStore, Mutation,
    Origin, PageId, PersonId, Request, Store, ViewId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

struct Home {
    workspace: Workspace,
    area: AreaId,
}

impl Home {
    fn new() -> Self {
        let workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut home = Self {
            workspace,
            area: AreaId::parse(&"0".repeat(32)).unwrap(),
        };
        let Some(Created::Area(area)) = home
            .edit(Mutation::CreateArea {
                name: "Private".into(),
            })
            .unwrap()
            .created
        else {
            panic!("an area")
        };
        home.area = area;
        home
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: Actor::Person(PersonId::new("benni")),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn page(&mut self, title: &str) -> PageId {
        let create = Mutation::CreatePage {
            area: self.area.clone(),
            parent: None,
            title: title.into(),
        };
        match self.edit(create).unwrap().created {
            Some(Created::Page(page)) => page,
            other => panic!("a Page, got {other:?}"),
        }
    }

    fn text(&mut self, page: &PageId, text: &str) -> (BlockId, ViewId) {
        let add = Mutation::AddTextBlock {
            page: page.clone(),
            after: None,
            text: text.into(),
        };
        match self.edit(add).unwrap().created {
            Some(Created::Block { block, view }) => (block, view),
            other => panic!("a Text Block, got {other:?}"),
        }
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
    let add = Mutation::AddDatabaseBlock {
        page: budget.clone(),
        after: None,
        name: "Expenses".into(),
        layout: liquid_core::Layout::Table,
    };
    let Some(Created::Block { block: table, .. }) = home.edit(add).unwrap().created else {
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
    home.edit(Mutation::RemoveView {
        page: trip.clone(),
        view,
    })
    .unwrap();
    assert!(home.ids("alfama", Found::Text).is_empty());
}

#[test]
fn a_replica_without_its_index_builds_it_once_opened() {
    let mut home = Home::new();
    let trip = home.page("Trip to Lisbon");
    home.text(&trip, "We fly on 12 November.");
    let mut store = home.workspace.into_store();
    store.drop_index();
    let mut workspace = Workspace::open(store).unwrap();
    let hits = workspace.search("novem", Found::Text, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(workspace.search("lisb", Found::Page, 10).unwrap().len(), 1);
    let long = "x".repeat(liquid_core::MAX_FILTER_TEXT + 1);
    assert_eq!(
        workspace.search(&long, Found::Text, 10),
        Err(Error::TooLong)
    );
}

#[test]
fn an_unreadable_page_neither_stops_the_open_nor_the_rest_of_the_index() {
    let mut home = Home::new();
    let trip = home.page("Trip to Lisbon");
    home.text(&trip, "We fly on 12 November.");
    let broken = home.page("Broken");
    home.text(&broken, "Never read");
    let mut store = home.workspace.into_store();
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
    let mut home = Home::new();
    home.page("Trip to Lisbon");
    let area = home.area.clone();
    let mut store = home.workspace.into_store();
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
