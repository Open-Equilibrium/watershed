//! Whiteboard Blocks: strokes, shapes and text as elements of their Block's entry in the
//! home Page's document, each written through the one pipeline.

// Integration tests are test code, but clippy.toml's exemption covers only #[test] items.
#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, BlockId, BlockKind, Content, Created, Element, ElementChange, ElementId, Error,
    Frame, InkColor, MAX_BOARD_POINTS, MAX_ELEMENTS, MAX_POINTS, MemoryStore, Mutation, Origin,
    PageId, Pen, PersonId, Request, Shape, WhiteboardStyle,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

/// A Workspace with one Page holding an empty Whiteboard.
fn board() -> (Workspace, PageId, BlockId) {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let area = match apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Personal".into(),
        },
    )
    .unwrap()
    .created
    {
        Some(Created::Area(area)) => area,
        other => panic!("an area, not {other:?}"),
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Trip to Lisbon".into(),
    };
    let Some(Created::Page(page)) = apply(&mut workspace, create).unwrap().created else {
        panic!("a Page")
    };
    let add = Mutation::AddWhiteboardBlock {
        page: page.clone(),
        after: None,
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add).unwrap().created else {
        panic!("a Whiteboard Block")
    };
    (workspace, page, block)
}

/// A stroke's points as the core stores them: x, y and pressure as little-endian f32.
fn points(points: &[(f32, f32, f32)]) -> Vec<u8> {
    points
        .iter()
        .flat_map(|(x, y, pressure)| [x, y, pressure])
        .flat_map(|value| value.to_le_bytes())
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
    Element {
        frame: frame(
            f64::from(xs[0]),
            10.0,
            f64::from(xs[xs.len() - 1] - xs[0]),
            0.0,
        ),
        color: InkColor::Data3,
        width: 1,
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
        content: Content::Text { text: text.into() },
    }
}

fn edit(
    page: &PageId,
    block: &BlockId,
    add: Vec<Element>,
    change: Vec<ElementChange>,
    remove: Vec<ElementId>,
) -> Mutation {
    Mutation::EditWhiteboard {
        page: page.clone(),
        block: block.clone(),
        add,
        change,
        remove,
    }
}

fn add(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    add: Vec<Element>,
) -> Vec<ElementId> {
    match apply(workspace, edit(page, block, add, vec![], vec![])) {
        Ok(Applied {
            created: Some(Created::Elements(ids)),
            ..
        }) => ids,
        other => panic!("elements, not {other:?}"),
    }
}

/// The board's elements, bottom to top.
fn elements(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
) -> Vec<(ElementId, Element)> {
    workspace.whiteboard(page, block).unwrap().unwrap().elements
}

#[test]
fn a_whiteboard_keeps_its_elements_in_drawing_order() {
    let (mut workspace, page, block) = board();
    assert_eq!(
        workspace.views(&page, 0, 10).unwrap()[0].kind,
        BlockKind::Whiteboard
    );
    let board = workspace.whiteboard(&page, &block).unwrap().unwrap();
    assert_eq!(
        board.style,
        WhiteboardStyle::HandDrawn,
        "hand-drawn by default"
    );
    assert!(board.elements.is_empty());

    let first = add(
        &mut workspace,
        &page,
        &block,
        vec![stroke(&[10.0, 20.0, 30.0]), rectangle()],
    );
    let second = add(&mut workspace, &page, &block, vec![text("Lisbon")]);
    workspace.close_page(&page);
    let read = elements(&mut workspace, &page, &block);
    assert_eq!(
        read.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        [first, second].concat(),
        "each new element on top, also after the Page loads again"
    );
    assert_eq!(
        read.into_iter()
            .map(|(_, element)| element)
            .collect::<Vec<_>>(),
        [stroke(&[10.0, 20.0, 30.0]), rectangle(), text("Lisbon")]
    );
}

#[test]
fn a_change_moves_and_restyles_an_element_but_never_redraws_its_ink() {
    let (mut workspace, page, block) = board();
    let ids = add(
        &mut workspace,
        &page,
        &block,
        vec![stroke(&[10.0, 20.0]), rectangle(), text("Lisbon")],
    );
    let change = |id: &ElementId, element: &Element| ElementChange {
        id: id.clone(),
        frame: Frame {
            rotation: 0.5,
            ..frame(5.0, 6.0, 7.0, 8.0)
        },
        color: InkColor::Data2,
        width: 2,
        text: matches!(element.content, Content::Text { .. }).then(|| "Porto".to_string()),
        arrowhead: None,
    };
    let originals = [stroke(&[10.0, 20.0]), rectangle(), text("Lisbon")];
    let changes = ids
        .iter()
        .zip(&originals)
        .map(|(id, element)| change(id, element))
        .collect();
    apply(&mut workspace, edit(&page, &block, vec![], changes, vec![])).unwrap();
    let read = elements(&mut workspace, &page, &block);
    for ((_, element), original) in read.iter().zip(&originals) {
        assert_eq!(
            element.frame,
            Frame {
                rotation: 0.5,
                ..frame(5.0, 6.0, 7.0, 8.0)
            }
        );
        assert_eq!(element.color, InkColor::Data2);
        assert_eq!(element.width, 2);
        if let Content::Stroke { points, .. } = &original.content {
            assert_eq!(
                element.content,
                Content::Stroke {
                    pen: Pen::Fountain,
                    points: points.clone()
                },
                "ink keeps its points"
            );
        }
    }
    assert_eq!(
        read[2].1.content,
        Content::Text {
            text: "Porto".into()
        }
    );

    let mut wrong = change(&ids[0], &originals[0]);
    wrong.text = Some("ink is no text".into());
    assert_eq!(
        apply(
            &mut workspace,
            edit(&page, &block, vec![], vec![wrong], vec![])
        ),
        Err(Error::WrongKind)
    );
    let mut arrow = change(&ids[1], &originals[1]);
    arrow.arrowhead = Some(true);
    assert_eq!(
        apply(
            &mut workspace,
            edit(&page, &block, vec![], vec![arrow], vec![])
        ),
        Err(Error::WrongKind),
        "only a line takes an arrowhead"
    );
    let missing = ElementId::parse(&"0".repeat(32)).unwrap();
    assert_eq!(
        apply(
            &mut workspace,
            edit(
                &page,
                &block,
                vec![],
                vec![change(&missing, &originals[0])],
                vec![]
            )
        ),
        Err(Error::NotFound)
    );
}

#[test]
fn erasing_part_of_a_stroke_replaces_it_by_its_pieces_in_one_action() {
    let (mut workspace, page, block) = board();
    let ids = add(
        &mut workspace,
        &page,
        &block,
        vec![rectangle(), stroke(&[0.0, 10.0, 20.0, 30.0])],
    );
    let before = workspace.history(None, 100).unwrap().len();
    let pieces = vec![stroke(&[0.0, 10.0]), stroke(&[20.0, 30.0])];
    let applied = apply(
        &mut workspace,
        edit(&page, &block, pieces.clone(), vec![], vec![ids[1].clone()]),
    )
    .unwrap();
    assert_eq!(workspace.history(None, 100).unwrap().len(), before + 1);
    let Some(Created::Elements(new)) = applied.created else {
        panic!("the pieces")
    };
    let read = elements(&mut workspace, &page, &block);
    assert_eq!(
        read,
        [
            vec![(ids[0].clone(), rectangle())],
            new.into_iter().zip(pieces).collect()
        ]
        .concat()
    );
    assert_eq!(
        apply(
            &mut workspace,
            edit(&page, &block, vec![], vec![], vec![ids[1].clone()])
        ),
        Err(Error::NotFound),
        "a removed element stays removed"
    );
}

#[test]
fn the_style_belongs_to_the_whole_board() {
    let (mut workspace, page, block) = board();
    let set = |style| Mutation::SetWhiteboardStyle {
        page: page.clone(),
        block: block.clone(),
        style,
    };
    apply(&mut workspace, set(WhiteboardStyle::Clean)).unwrap();
    workspace.close_page(&page);
    assert_eq!(
        workspace.whiteboard(&page, &block).unwrap().unwrap().style,
        WhiteboardStyle::Clean
    );
    assert_eq!(
        apply(&mut workspace, set(WhiteboardStyle::Clean)),
        Err(Error::NoChange)
    );
}

#[test]
fn elements_are_checked_before_they_are_written() {
    let (mut workspace, page, block) = board();
    let refused = |workspace: &mut Workspace, element: Element| {
        apply(
            workspace,
            edit(&page, &block, vec![element], vec![], vec![]),
        )
    };
    let mut odd = stroke(&[1.0, 2.0]);
    odd.content = Content::Stroke {
        pen: Pen::Ball,
        points: vec![0; 13],
    };
    assert_eq!(
        refused(&mut workspace, odd),
        Err(Error::WrongKind),
        "12 bytes a point"
    );
    let mut nowhere = rectangle();
    nowhere.frame.x = f64::NAN;
    assert_eq!(refused(&mut workspace, nowhere), Err(Error::WrongKind));
    let mut thick = rectangle();
    thick.width = 3;
    assert_eq!(refused(&mut workspace, thick), Err(Error::WrongKind));
    let mut corners = rectangle();
    corners.content = Content::Shape {
        shape: Shape::Triangle {
            corners: [0.5, 0.0, 1.0, 1.0, 0.0, 2.0],
        },
        seed: 1,
    };
    assert_eq!(
        refused(&mut workspace, corners),
        Err(Error::WrongKind),
        "a triangle's corners lie in its frame"
    );
    let long: Vec<f32> = (0..=MAX_POINTS).map(|x| x as f32).collect();
    assert_eq!(refused(&mut workspace, stroke(&long)), Err(Error::TooMany));
    let text_block = Mutation::EditText {
        page: page.clone(),
        block: block.clone(),
        at: 0,
        delete: 0,
        insert: "x".into(),
    };
    assert_eq!(apply(&mut workspace, text_block), Err(Error::NotFound));
    assert!(
        elements(&mut workspace, &page, &block).is_empty(),
        "nothing written"
    );
}

#[test]
fn a_board_holds_a_bounded_number_of_elements_and_points() {
    let (mut workspace, page, block) = board();
    add(
        &mut workspace,
        &page,
        &block,
        vec![rectangle(); MAX_ELEMENTS - 1],
    );
    add(&mut workspace, &page, &block, vec![rectangle()]);
    assert_eq!(
        apply(
            &mut workspace,
            edit(&page, &block, vec![rectangle()], vec![], vec![])
        ),
        Err(Error::TooMany)
    );

    let (mut workspace, page, block) = board();
    let longest: Vec<f32> = (0..MAX_POINTS).map(|x| x as f32).collect();
    let strokes = MAX_BOARD_POINTS / MAX_POINTS;
    add(
        &mut workspace,
        &page,
        &block,
        vec![stroke(&longest); strokes],
    );
    assert_eq!(
        apply(
            &mut workspace,
            edit(&page, &block, vec![stroke(&[1.0])], vec![], vec![])
        ),
        Err(Error::TooMany),
        "the points of all its strokes are bounded too, so the board reads bounded"
    );
}
