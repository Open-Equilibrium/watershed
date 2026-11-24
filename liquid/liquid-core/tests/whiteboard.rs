#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Applied, BlockId, BlockKind, Content, Created, Drawing, DrawingStyle, Element, ElementChange,
    ElementId, Error, Frame, InkColor, MAX_BOARD_POINTS, MAX_ELEMENTS, MAX_POINTS, Mutation,
    PageId, Pen, Shape,
};

struct Board {
    workspace: Workspace,
    page: PageId,
    block: BlockId,
}

impl Board {
    fn new() -> Self {
        let mut workspace = workspace();
        let Created::Area(area) = created(
            &mut workspace,
            Mutation::CreateArea {
                name: "Personal".into(),
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
        let Created::Block { block, .. } = created(
            &mut workspace,
            Mutation::AddWhiteboardBlock {
                page: page.clone(),
                after: None,
            },
        ) else {
            panic!("a Whiteboard Block")
        };
        Self {
            workspace,
            page,
            block,
        }
    }

    fn edit(
        &mut self,
        add: Vec<Element>,
        change: Vec<ElementChange>,
        remove: Vec<ElementId>,
    ) -> Result<Applied, Error> {
        apply(
            &mut self.workspace,
            Mutation::EditDrawing {
                page: self.page.clone(),
                block: self.block.clone(),
                add,
                change,
                remove,
            },
        )
    }

    fn add(&mut self, add: Vec<Element>) -> Vec<ElementId> {
        let Some(Created::Elements(ids)) = self.edit(add, vec![], vec![]).unwrap().created else {
            panic!("elements")
        };
        ids
    }

    fn drawing(&mut self) -> Drawing {
        let drawing = self.workspace.drawing(&self.page, &self.block, 0..1);
        drawing.unwrap().unwrap()
    }

    fn elements(&mut self) -> Vec<(ElementId, Element)> {
        self.drawing().elements
    }
}

fn points(points: &[(f32, f32, f32)]) -> Vec<u8> {
    points
        .iter()
        .flat_map(|&(x, y, pressure)| [x, y, pressure])
        .flat_map(f32::to_le_bytes)
        .collect()
}

fn frame(x: f64, y: f64, width: f64, height: f64) -> Frame {
    Frame {
        x,
        y,
        width,
        height,
        rotation: 0.0,
    }
}

fn stroke(xs: &[f32]) -> Element {
    let drawn: Vec<_> = xs.iter().map(|x| (*x, 10.0, 0.5)).collect();
    let span = f64::from(xs[xs.len() - 1] - xs[0]);
    Element {
        frame: frame(f64::from(xs[0]), 10.0, span, 0.0),
        color: InkColor::Data3,
        width: 1,
        doc_page: 0,
        content: Content::Stroke {
            pen: Pen::Fountain,
            points: points(&drawn),
        },
    }
}

fn rectangle() -> Element {
    Element {
        frame: frame(40.0, 66.0, 150.0, 70.0),
        color: InkColor::Data1,
        width: 1,
        doc_page: 0,
        content: Content::Shape {
            shape: Shape::Rectangle,
            seed: 7,
        },
    }
}

fn text(text: &str) -> Element {
    Element {
        frame: frame(115.0, 97.0, 120.0, 24.0),
        color: InkColor::Text,
        width: 1,
        doc_page: 0,
        content: Content::Text { text: text.into() },
    }
}

#[test]
fn a_whiteboard_keeps_its_style_and_its_elements_in_drawing_order() {
    let mut board = Board::new();
    let kind = board.workspace.views(&board.page, 0, 10).unwrap()[0].kind;
    assert_eq!(kind, BlockKind::Whiteboard);
    assert_eq!(board.drawing().style, DrawingStyle::HandDrawn);
    assert!(board.elements().is_empty());

    let first = board.add(vec![stroke(&[10.0, 20.0, 30.0]), rectangle()]);
    let second = board.add(vec![text("Lisbon")]);
    board.workspace.close_page(&board.page);
    let read = board.elements();
    let ids: Vec<_> = read.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(ids, [first, second].concat(), "kept after reopening");
    let drawn: Vec<_> = read.into_iter().map(|(_, element)| element).collect();
    let expected = [stroke(&[10.0, 20.0, 30.0]), rectangle(), text("Lisbon")];
    assert_eq!(drawn, expected);

    let set = Mutation::SetDrawingStyle {
        page: board.page.clone(),
        block: board.block.clone(),
        style: DrawingStyle::Clean,
    };
    apply(&mut board.workspace, set.clone()).unwrap();
    board.workspace.close_page(&board.page);
    assert_eq!(board.drawing().style, DrawingStyle::Clean);
    assert_eq!(apply(&mut board.workspace, set), Err(Error::NoChange));
}

#[test]
fn a_change_moves_and_restyles_an_element_but_never_redraws_its_ink() {
    let mut board = Board::new();
    let originals = [stroke(&[10.0, 20.0]), rectangle(), text("Lisbon")];
    let ids = board.add(originals.to_vec());
    let moved = Frame {
        rotation: 0.5,
        ..frame(5.0, 6.0, 7.0, 8.0)
    };
    let change = |id: &ElementId, element: &Element| ElementChange {
        id: id.clone(),
        frame: moved,
        color: InkColor::Data2,
        width: 2,
        text: matches!(element.content, Content::Text { .. }).then(|| "Porto".to_string()),
        arrowhead: None,
    };
    let changes = ids
        .iter()
        .zip(&originals)
        .map(|(id, element)| change(id, element))
        .collect();
    board.edit(vec![], changes, vec![]).unwrap();
    let read = board.elements();
    for ((_, element), original) in read.iter().zip(&originals) {
        assert_eq!(element.frame, moved);
        assert_eq!(element.color, InkColor::Data2);
        assert_eq!(element.width, 2);
        if let Content::Stroke { .. } = original.content {
            assert_eq!(element.content, original.content, "ink keeps its points");
        }
    }
    assert_eq!(read[2].1.content, text("Porto").content);

    let mut wrong = change(&ids[0], &originals[0]);
    wrong.text = Some("ink is no text".into());
    let refused = board.edit(vec![], vec![wrong], vec![]);
    assert_eq!(refused, Err(Error::WrongKind));
    let mut arrow = change(&ids[1], &originals[1]);
    arrow.arrowhead = Some(true);
    let refused = board.edit(vec![], vec![arrow], vec![]);
    assert_eq!(refused, Err(Error::WrongKind), "arrowheads need lines");
    let missing = ElementId::parse(&"0".repeat(32)).unwrap();
    let unknown = change(&missing, &originals[0]);
    let refused = board.edit(vec![], vec![unknown], vec![]);
    assert_eq!(refused, Err(Error::NotFound));
}

#[test]
fn erasing_part_of_a_stroke_replaces_it_by_its_pieces_in_one_action() {
    let mut board = Board::new();
    let ids = board.add(vec![rectangle(), stroke(&[0.0, 10.0, 20.0, 30.0])]);
    let before = board.workspace.history(None, 100).unwrap().len();
    let pieces = vec![stroke(&[0.0, 10.0]), stroke(&[20.0, 30.0])];
    let Some(Created::Elements(new)) = board
        .edit(pieces.clone(), vec![], vec![ids[1].clone()])
        .unwrap()
        .created
    else {
        panic!("the pieces")
    };
    let after = board.workspace.history(None, 100).unwrap().len();
    assert_eq!(after, before + 1);
    let expected = [
        vec![(ids[0].clone(), rectangle())],
        new.into_iter().zip(pieces).collect(),
    ]
    .concat();
    assert_eq!(board.elements(), expected);
    let refused = board.edit(vec![], vec![], vec![ids[1].clone()]);
    assert_eq!(refused, Err(Error::NotFound), "stays removed");
}

#[test]
fn elements_are_checked_before_they_are_written() {
    let mut board = Board::new();
    let mut odd = stroke(&[1.0, 2.0]);
    odd.content = Content::Stroke {
        pen: Pen::Ball,
        points: vec![0; 13],
    };
    let mut nowhere = rectangle();
    nowhere.frame.x = f64::NAN;
    let mut thick = rectangle();
    thick.width = 3;
    let mut corners = rectangle();
    corners.content = Content::Shape {
        shape: Shape::Triangle {
            corners: [0.5, 0.0, 1.0, 1.0, 0.0, 2.0],
        },
        seed: 1,
    };
    for (element, why) in [
        (odd, "12 bytes a point"),
        (nowhere, "a NaN frame"),
        (thick, "a width of 3"),
        (corners, "a triangle's corners lie in its frame"),
    ] {
        let refused = board.edit(vec![element], vec![], vec![]);
        assert_eq!(refused, Err(Error::WrongKind), "{why}");
    }
    let long: Vec<f32> = (0..=MAX_POINTS).map(|x| x as f32).collect();
    let refused = board.edit(vec![stroke(&long)], vec![], vec![]);
    assert_eq!(refused, Err(Error::TooMany));
    let text_block = Mutation::EditText {
        page: board.page.clone(),
        block: board.block.clone(),
        at: 0,
        delete: 0,
        insert: "x".into(),
    };
    let refused = apply(&mut board.workspace, text_block);
    assert_eq!(refused, Err(Error::NotFound));
    assert!(board.elements().is_empty(), "nothing written");
}

#[test]
fn a_board_holds_a_bounded_number_of_elements_and_points() {
    let mut board = Board::new();
    board.add(vec![rectangle(); MAX_ELEMENTS - 1]);
    board.add(vec![rectangle()]);
    let refused = board.edit(vec![rectangle()], vec![], vec![]);
    assert_eq!(refused, Err(Error::TooMany));

    let mut board = Board::new();
    let longest: Vec<f32> = (0..MAX_POINTS).map(|x| x as f32).collect();
    board.add(vec![stroke(&longest); MAX_BOARD_POINTS / MAX_POINTS]);
    let refused = board.edit(vec![stroke(&[1.0])], vec![], vec![]);
    assert_eq!(refused, Err(Error::TooMany));
}
