#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Applied, Created, Error, Layout, Mutation, PageId, TextStyle, Tile, View, ViewId,
};

struct Page {
    workspace: Workspace,
    page: PageId,
}

impl Page {
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
        let Created::Page(page) = created(
            &mut workspace,
            Mutation::CreatePage {
                area,
                parent: None,
                title: "Trip to Lisbon".into(),
            },
        ) else {
            panic!("a Page")
        };
        Self { workspace, page }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn block(&mut self, add: Mutation) -> ViewId {
        let Created::Block { view, .. } = created(&mut self.workspace, add) else {
            panic!("a Block")
        };
        view
    }

    fn text(&mut self, after: Option<&ViewId>, text: &str) -> ViewId {
        self.block(Mutation::AddTextBlock {
            page: self.page.clone(),
            after: after.cloned(),
            text: text.into(),
        })
    }

    fn board(&mut self, after: Option<&ViewId>) -> ViewId {
        self.block(Mutation::AddWhiteboardBlock {
            page: self.page.clone(),
            after: after.cloned(),
        })
    }

    fn tile(&mut self, view: &ViewId, tile: Option<Tile>) -> Result<Applied, Error> {
        self.edit(Mutation::SetTile {
            page: self.page.clone(),
            view: view.clone(),
            tile,
        })
    }

    fn inner(&mut self, view: &ViewId, tile: Option<Tile>) -> Result<Applied, Error> {
        self.edit(Mutation::SetInnerTile {
            page: self.page.clone(),
            view: view.clone(),
            tile,
        })
    }

    fn move_views(
        &mut self,
        first: &ViewId,
        last: &ViewId,
        after: Option<&ViewId>,
    ) -> Result<Applied, Error> {
        self.edit(Mutation::MoveViews {
            page: self.page.clone(),
            first: first.clone(),
            last: last.clone(),
            after: after.cloned(),
        })
    }

    fn boundary(&mut self, view: &ViewId, boundary: bool) -> Result<Applied, Error> {
        self.edit(Mutation::SetSectionBoundary {
            page: self.page.clone(),
            view: view.clone(),
            boundary,
        })
    }

    fn reopened(&mut self) -> Vec<View> {
        self.workspace.close_page(&self.page);
        self.workspace.views(&self.page, 0, 100).unwrap()
    }

    fn tiles(&mut self) -> Vec<Option<Tile>> {
        self.reopened().iter().map(|view| view.tile).collect()
    }

    fn order(&mut self) -> Vec<ViewId> {
        self.reopened().into_iter().map(|view| view.id).collect()
    }

    fn arranged(&mut self) -> bool {
        self.workspace.arranged(&self.page).unwrap()
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
    assert_eq!(page.tiles(), [tile(0, 5), None, tile(5, 7)]);

    let refused = page.tile(&flight, tile(0, 6));
    assert_eq!(refused, Err(Error::NotAllowed), "first Text Block only");
    for wrong in [tile(0, 0), tile(0, 13), tile(8, 5)] {
        assert_eq!(page.tile(&route, wrong), Err(Error::OutOfRange));
    }
    assert!(page.arranged());
    page.tile(&plan, None).unwrap();
    page.tile(&route, None).unwrap();
    assert_eq!(page.tiles(), [None, None, None]);
    assert!(!page.arranged());
}

#[test]
fn a_section_s_text_blocks_place_themselves_in_its_columns_and_arrange_the_page() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let route = page.board(Some(&flight));
    page.inner(&plan, tile(0, 3)).unwrap();
    page.inner(&flight, tile(3, 3)).unwrap();
    let refused = page.inner(&route, tile(0, 3));
    assert_eq!(refused, Err(Error::WrongKind));
    let inner: Vec<_> = page.reopened().iter().map(|view| view.inner).collect();
    assert_eq!(inner, [tile(0, 3), tile(3, 3), None]);
    assert!(page.arranged());
}

#[test]
fn a_run_of_views_moves_after_another_or_to_the_start() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let route = page.board(Some(&flight));
    let ideas = page.text(Some(&route), "Ideas");
    page.move_views(&plan, &flight, Some(&route)).unwrap();
    assert_eq!(
        page.order(),
        [route.clone(), plan.clone(), flight.clone(), ideas.clone()]
    );
    page.move_views(&ideas, &ideas, None).unwrap();
    let expected = [ideas, route.clone(), plan.clone(), flight.clone()];
    assert_eq!(page.order(), expected);
    let refused = page.move_views(&plan, &flight, Some(&route));
    assert_eq!(refused, Err(Error::NoChange));
    let refused = page.move_views(&plan, &flight, Some(&flight));
    assert_eq!(refused, Err(Error::OutOfRange));
    let refused = page.move_views(&flight, &plan, None);
    assert_eq!(refused, Err(Error::OutOfRange));
}

#[test]
fn a_split_section_keeps_the_width_and_a_join_keeps_the_upper_place() {
    let mut page = Page::new();
    let plan = page.text(None, "Plan");
    let flight = page.text(Some(&plan), "We fly on 12 November.");
    let hotel = page.text(Some(&flight), "The flat is in Alfama.");
    page.tile(&plan, tile(0, 5)).unwrap();
    page.boundary(&flight, true).unwrap();
    assert_eq!(page.tiles(), [tile(0, 5), tile(0, 5), None]);
    page.tile(&flight, tile(6, 6)).unwrap();
    page.boundary(&flight, false).unwrap();
    assert_eq!(page.tiles(), [tile(0, 5), None, None]);
    page.board(Some(&flight));
    assert_eq!(page.tiles(), [tile(0, 5), None, None, tile(0, 5)]);
    assert_eq!(page.reopened()[3].id, hotel);
}

#[test]
fn a_section_keeps_its_tile_when_its_first_line_changes() {
    let mut page = Page::new();
    let board = page.board(None);
    let ideas = page.text(Some(&board), "Ideas");
    let sintra = page.text(Some(&ideas), "Day trip to Sintra.");
    page.tile(&board, tile(0, 6)).unwrap();
    page.tile(&ideas, tile(6, 6)).unwrap();
    let block = page.reopened()[1].block.clone();
    let split = Mutation::SplitTextBlock {
        page: page.page.clone(),
        block,
        view: ideas.clone(),
        at: 0,
        style: TextStyle::Paragraph,
    };
    page.edit(split).unwrap();
    assert_eq!(page.tiles(), [tile(0, 6), tile(6, 6), None, None]);

    let lead = page.reopened()[1].id.clone();
    let remove = Mutation::RemoveView {
        page: page.page.clone(),
        view: lead,
    };
    page.edit(remove).unwrap();
    assert_eq!(page.tiles(), [tile(0, 6), tile(6, 6), None]);

    let block = page.reopened()[1].block.clone();
    let code = Mutation::TurnIntoCode {
        page: page.page.clone(),
        block,
    };
    page.edit(code).unwrap();
    let views = page.reopened();
    assert_eq!(views[2].id, sintra);
    assert_eq!(views[2].tile, tile(6, 6));
}

#[test]
fn deleting_a_table_joins_the_sections_around_its_view() {
    let mut page = Page::new();
    let above = page.text(None, "Plan");
    let add = Mutation::AddDatabaseBlock {
        page: page.page.clone(),
        after: Some(above),
        name: "Costs".into(),
        layout: Layout::Table,
    };
    let Created::Block { block: table, view } = created(&mut page.workspace, add) else {
        panic!("a Table")
    };
    let below = page.text(Some(&view), "Ideas");
    page.tile(&below, tile(6, 6)).unwrap();
    page.edit(Mutation::DeleteTable { block: table }).unwrap();
    assert_eq!(page.tiles(), [None, None]);
    assert!(!page.arranged());
}
