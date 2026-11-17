//! Annotating Media Blocks (ADR-0307): a PDF's or image's drawing is kept in the Media
//! Block's entry, page by page, with the Whiteboard's elements and the highlighter.

// Integration tests are test code, but clippy.toml's exemption covers only #[test] items.
#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, BlockId, Content, Created, DrawingStyle, Element, Error, Frame, InkColor,
    MAX_DOC_PAGES, MAX_ELEMENTS, MemoryStore, Mutation, Origin, PageId, Pen, PersonId, Request,
    Shape,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

fn created(workspace: &mut Workspace, mutation: Mutation) -> Created {
    apply(workspace, mutation).unwrap().created.unwrap()
}

/// A Page holding a PDF's Media Block and a Whiteboard.
fn agreement() -> (Workspace, PageId, BlockId, BlockId) {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
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
    let owner = Actor::Person(PersonId::new("benni"));
    let mut upload = workspace.begin_upload(&owner, 7).unwrap();
    workspace.upload(&mut upload, b"%PDF-1.").unwrap();
    let attachment = workspace.finish_upload(upload).unwrap();
    let Created::Block { block: pdf, .. } = created(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: None,
            name: "Rental agreement.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.id,
        },
    ) else {
        panic!("a Media Block")
    };
    let Created::Block { block: board, .. } = created(
        &mut workspace,
        Mutation::AddWhiteboardBlock {
            page: page.clone(),
            after: None,
        },
    ) else {
        panic!("a Whiteboard")
    };
    (workspace, page, pdf, board)
}

fn mark(doc_page: u32, color: InkColor) -> Element {
    let points: Vec<u8> = [10.0f32, 20.0, 0.5, 200.0, 20.0, 0.5]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    Element {
        frame: Frame {
            x: 10.0,
            y: 20.0,
            width: 190.0,
            height: 0.0,
            rotation: 0.0,
        },
        color,
        width: 1,
        doc_page,
        content: Content::Stroke {
            pen: Pen::Highlighter,
            points,
        },
    }
}

fn box_on(doc_page: u32) -> Element {
    Element {
        frame: Frame {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            rotation: 0.0,
        },
        color: InkColor::Text,
        width: 1,
        doc_page,
        content: Content::Shape {
            shape: Shape::Rectangle,
            seed: 1,
        },
    }
}

fn draw(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    add: Vec<Element>,
) -> Result<Applied, Error> {
    apply(
        workspace,
        Mutation::EditDrawing {
            page: page.clone(),
            block: block.clone(),
            add,
            change: Vec::new(),
            remove: Vec::new(),
        },
    )
}

#[test]
fn a_pdf_keeps_its_drawing_page_by_page_and_reads_a_window_of_pages() {
    let (mut workspace, page, pdf, _) = agreement();
    let drawing = workspace.drawing(&page, &pdf, 0..10).unwrap().unwrap();
    assert_eq!(drawing.style, DrawingStyle::HandDrawn);
    assert!(drawing.elements.is_empty(), "an unannotated file has none");

    draw(
        &mut workspace,
        &page,
        &pdf,
        vec![
            mark(0, InkColor::Yellow),
            mark(3, InkColor::Pink),
            box_on(3),
        ],
    )
    .unwrap();
    workspace.close_page(&page);
    let window = workspace.drawing(&page, &pdf, 2..4).unwrap().unwrap();
    assert_eq!(
        window
            .elements
            .iter()
            .map(|(_, element)| (element.doc_page, element.color))
            .collect::<Vec<_>>(),
        [(3, InkColor::Pink), (3, InkColor::Text)],
        "only the pages asked for, bottom to top"
    );
    let first = workspace.drawing(&page, &pdf, 0..1).unwrap().unwrap();
    assert_eq!(first.elements.len(), 1);
    assert_eq!(first.elements[0].1, mark(0, InkColor::Yellow));
    assert_eq!(
        workspace.media(&page, &pdf).unwrap().unwrap().name,
        "Rental agreement.pdf",
        "the file stays as it is"
    );
}

#[test]
fn each_page_holds_the_board_caps_and_a_document_its_pages() {
    let (mut workspace, page, pdf, board) = agreement();
    draw(&mut workspace, &page, &pdf, vec![box_on(0); MAX_ELEMENTS]).unwrap();
    assert_eq!(
        draw(&mut workspace, &page, &pdf, vec![box_on(0)]),
        Err(Error::TooMany),
        "a page is full"
    );
    draw(&mut workspace, &page, &pdf, vec![box_on(1)]).expect("the next page is not");
    assert_eq!(
        draw(&mut workspace, &page, &pdf, vec![box_on(MAX_DOC_PAGES)]),
        Err(Error::WrongKind),
        "past the last page a document may hold ink on"
    );
    assert_eq!(
        draw(&mut workspace, &page, &board, vec![box_on(1)]),
        Err(Error::WrongKind),
        "a Whiteboard has one page"
    );
}

#[test]
fn actions_recorded_under_the_whiteboard_names_still_read() {
    let stored = r#"{"EditWhiteboard":{"page":"p","block":"b","add":[],"change":[],"remove":[]}}"#;
    let read: Mutation = serde_json::from_str(stored).unwrap();
    assert!(matches!(read, Mutation::EditDrawing { .. }));
    let stored = r#"{"SetWhiteboardStyle":{"page":"p","block":"b","style":"Clean"}}"#;
    let read: Mutation = serde_json::from_str(stored).unwrap();
    assert!(matches!(
        read,
        Mutation::SetDrawingStyle {
            style: DrawingStyle::Clean,
            ..
        }
    ));
}

#[test]
fn a_read_across_pages_stays_within_one_board_s_points() {
    let (mut workspace, page, pdf, _) = agreement();
    let longest: Vec<u8> = (0..liquid_core::MAX_POINTS)
        .flat_map(|at| [at as f32, 1.0, 0.5])
        .flat_map(f32::to_le_bytes)
        .collect();
    let full = |doc_page| Element {
        content: Content::Stroke {
            pen: Pen::Fountain,
            points: longest.clone(),
        },
        ..mark(doc_page, InkColor::Text)
    };
    let per_page = liquid_core::MAX_BOARD_POINTS / liquid_core::MAX_POINTS;
    for doc_page in 0..2 {
        draw(&mut workspace, &page, &pdf, vec![full(doc_page); per_page]).unwrap();
    }
    let read = workspace.drawing(&page, &pdf, 0..2).unwrap().unwrap();
    let points: usize = read
        .elements
        .iter()
        .map(|(_, element)| match &element.content {
            Content::Stroke { points, .. } => points.len() / 12,
            _ => 0,
        })
        .sum();
    assert!(
        points <= liquid_core::MAX_BOARD_POINTS,
        "one read carries at most a board's points, not {points}"
    );
    assert!(
        read.elements
            .iter()
            .all(|(_, element)| element.doc_page == 0)
    );
}
