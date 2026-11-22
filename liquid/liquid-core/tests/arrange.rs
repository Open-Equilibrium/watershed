#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, Created, Error, MemoryStore, Mutation, Origin, PageId, PersonId, Request, Tile,
    ViewId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

struct Page {
    workspace: Workspace,
    page: PageId,
}

impl Page {
    fn new() -> Self {
        let workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut page = Self {
            workspace,
            page: PageId::parse(&"0".repeat(32)).unwrap(),
        };
        let Some(Created::Area(area)) = page
            .edit(Mutation::CreateArea {
                name: "Private".into(),
            })
            .unwrap()
            .created
        else {
            panic!("an area")
        };
        let Some(Created::Page(id)) = page
            .edit(Mutation::CreatePage {
                area,
                parent: None,
                title: "Trip to Lisbon".into(),
            })
            .unwrap()
            .created
        else {
            panic!("a Page")
        };
        page.page = id;
        page
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: Actor::Person(PersonId::new("benni")),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn text(&mut self, after: Option<&ViewId>, text: &str) -> ViewId {
        let add = Mutation::AddTextBlock {
            page: self.page.clone(),
            after: after.cloned(),
            text: text.into(),
        };
        match self.edit(add).unwrap().created {
            Some(Created::Block { view, .. }) => view,
            other => panic!("a Text Block, got {other:?}"),
        }
    }

    fn board(&mut self, after: Option<&ViewId>) -> ViewId {
        let add = Mutation::AddWhiteboardBlock {
            page: self.page.clone(),
            after: after.cloned(),
        };
        match self.edit(add).unwrap().created {
            Some(Created::Block { view, .. }) => view,
            other => panic!("a Whiteboard, got {other:?}"),
        }
    }

    fn tile(&mut self, view: &ViewId, tile: Option<Tile>) -> Result<Applied, Error> {
        self.edit(Mutation::SetTile {
            page: self.page.clone(),
            view: view.clone(),
            tile,
        })
    }

    fn reopened(&mut self) -> Vec<liquid_core::View> {
        self.workspace.close_page(&self.page);
        self.workspace.views(&self.page, 0, 100).unwrap()
    }

    fn order(&mut self) -> Vec<ViewId> {
        self.reopened().into_iter().map(|view| view.id).collect()
    }
}

fn tile(column: u8, span: u8) -> Option<Tile> {
    Some(Tile { column, span })
}

#[test]
fn a_block_and_a_section_keep_their_column_and_span() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let route = page.board(Some(&flight));
    page.tile(&plan, tile(0, 5)).unwrap();
    page.tile(&route, tile(5, 7)).unwrap();
    let views = page.reopened();
    let placed: Vec<_> = views.iter().map(|view| view.tile).collect();
    assert_eq!(placed, [tile(0, 5), None, tile(5, 7)]);

    assert_eq!(
        page.tile(&flight, tile(0, 6)),
        Err(Error::NotAllowed),
        "only a Section's first Text Block holds its tile"
    );
    for wrong in [tile(0, 0), tile(0, 13), tile(8, 5)] {
        assert_eq!(page.tile(&route, wrong), Err(Error::OutOfRange));
    }
    page.tile(&route, None).unwrap();
    assert_eq!(page.reopened()[2].tile, None);
}

#[test]
fn a_section_s_text_blocks_place_themselves_in_its_columns() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let route = page.board(Some(&flight));
    let inner = |view: &ViewId, tile| Mutation::SetInnerTile {
        page: page.page.clone(),
        view: view.clone(),
        tile,
    };
    let (first, second, board) = (
        inner(&plan, tile(0, 3)),
        inner(&flight, tile(3, 3)),
        inner(&route, tile(0, 3)),
    );
    page.edit(first).unwrap();
    page.edit(second).unwrap();
    assert_eq!(page.edit(board), Err(Error::WrongKind));
    let views = page.reopened();
    assert_eq!(
        views.iter().map(|view| view.inner).collect::<Vec<_>>(),
        [tile(0, 3), tile(3, 3), None]
    );
}

#[test]
fn a_run_of_views_moves_after_another_or_to_the_start() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let route = page.board(Some(&flight));
    let ideas = page.text(Some(&route), "Ideas");
    let moved = |first: &ViewId, last: &ViewId, after: Option<&ViewId>| Mutation::MoveViews {
        page: page.page.clone(),
        first: first.clone(),
        last: last.clone(),
        after: after.cloned(),
    };
    let (section_down, ideas_first, into_itself, backwards) = (
        moved(&plan, &flight, Some(&route)),
        moved(&ideas, &ideas, None),
        moved(&plan, &flight, Some(&flight)),
        moved(&flight, &plan, None),
    );
    page.edit(section_down).unwrap();
    assert_eq!(
        page.order(),
        [route.clone(), plan.clone(), flight.clone(), ideas.clone()]
    );
    page.edit(ideas_first).unwrap();
    assert_eq!(
        page.order(),
        [ideas.clone(), route.clone(), plan.clone(), flight.clone()]
    );
    assert_eq!(page.edit(into_itself), Err(Error::OutOfRange));
    assert_eq!(page.edit(backwards), Err(Error::OutOfRange));
}
