//! Whiteboard Blocks and file cells as Dart builds and reads them (ADR-0305, ADR-0306):
//! elements cross as flat messages with a stroke's points as bytes, and a file cell names
//! finished attachments whose sizes the core reads.

use super::workspace::{Folder, Memory, lisbon, mutation, query};
use crate::api::database::{BridgeCell, BridgeCellKind, BridgeFieldKind};
use crate::api::media::BridgeMedia;
use crate::api::replica::LiquidWorkspace;
use crate::api::whiteboard::{
    BridgeElement, BridgeElementChange, BridgeElementKind, BridgeInkColor, BridgePen, BridgeShape,
    BridgeWhiteboardEdit, BridgeWhiteboardStyle,
};
use crate::api::workspace::{
    BridgeBlockKind, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind,
};

fn element(kind: BridgeElementKind) -> BridgeElement {
    BridgeElement {
        id: None,
        kind,
        x: 40.0,
        y: 66.0,
        w: 150.0,
        h: 70.0,
        rotation: 0.25,
        color: BridgeInkColor::Data1,
        width: 1,
        pen: BridgePen::Ball,
        points: Vec::new(),
        shape: BridgeShape::Rectangle,
        seed: 0,
        arrowhead: false,
        corners: Vec::new(),
        text: String::new(),
    }
}

#[test]
fn a_whiteboard_s_elements_cross_whole_and_come_back_as_drawn() {
    let folder = Folder::new("whiteboard");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, intro) = lisbon(&workspace);
    let added = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro),
            ..mutation(BridgeMutationKind::AddWhiteboardBlock)
        })
        .unwrap();
    let block = added.block.clone().unwrap();
    let ink: Vec<u8> = [1.5f32, 2.0, 0.25, 3.0, 4.0, 0.75]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let drawn = vec![
        BridgeElement {
            pen: BridgePen::Brush,
            points: ink,
            ..element(BridgeElementKind::Stroke)
        },
        BridgeElement {
            shape: BridgeShape::Triangle,
            corners: vec![0.5, 0.0, 1.0, 1.0, 0.0, 1.0],
            seed: 4_000_000_000,
            ..element(BridgeElementKind::Shape)
        },
        BridgeElement {
            shape: BridgeShape::Line,
            arrowhead: true,
            w: -20.0,
            ..element(BridgeElementKind::Shape)
        },
        BridgeElement {
            text: "Lisbon".into(),
            color: BridgeInkColor::Text,
            ..element(BridgeElementKind::Text)
        },
    ];
    let edit = |whiteboard: BridgeWhiteboardEdit| BridgeMutation {
        page: Some(page.clone()),
        block: Some(block.clone()),
        whiteboard: Some(whiteboard),
        ..mutation(BridgeMutationKind::EditWhiteboard)
    };
    let applied = workspace
        .apply(edit(BridgeWhiteboardEdit {
            add: drawn.clone(),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(applied.elements.len(), 4);
    let read = || {
        workspace
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.clone()),
                ..query(BridgeQueryKind::Whiteboard)
            })
            .unwrap()
            .whiteboard
            .unwrap()
    };
    let board = read();
    assert_eq!(board.style, BridgeWhiteboardStyle::HandDrawn);
    let expected: Vec<_> = drawn
        .into_iter()
        .zip(&applied.elements)
        .map(|(element, id)| BridgeElement {
            id: Some(id.clone()),
            ..element
        })
        .collect();
    assert_eq!(board.elements, expected);

    workspace
        .apply(edit(BridgeWhiteboardEdit {
            change: vec![BridgeElementChange {
                id: applied.elements[3].clone(),
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0,
                rotation: 0.0,
                color: BridgeInkColor::Data3,
                width: 2,
                text: Some("Porto".into()),
                arrowhead: None,
            }],
            remove: vec![applied.elements[0].clone()],
            ..Default::default()
        }))
        .unwrap();
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            whiteboard_style: Some(BridgeWhiteboardStyle::Clean),
            ..mutation(BridgeMutationKind::SetWhiteboardStyle)
        })
        .unwrap();
    let board = read();
    assert_eq!(board.style, BridgeWhiteboardStyle::Clean);
    assert_eq!(board.elements.len(), 3);
    assert_eq!(board.elements[2].text, "Porto");
    assert_eq!(board.elements[2].color, BridgeInkColor::Data3);
    let views = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    assert_eq!(views[1].kind, BridgeBlockKind::Whiteboard);
}

#[test]
fn a_file_cell_names_attachments_and_reads_their_sizes() {
    let folder = Folder::new("file-cell");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, intro) = lisbon(&workspace);
    let block = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro),
            text: Some("Bookings".into()),
            ..mutation(BridgeMutationKind::AddDatabaseBlock)
        })
        .unwrap()
        .block;
    let field = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: block.clone(),
            last: true,
            text: Some("Files".into()),
            field_kind: Some(BridgeFieldKind::File),
            ..mutation(BridgeMutationKind::AddField)
        })
        .unwrap()
        .field;
    let row = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: block.clone(),
            last: true,
            ..mutation(BridgeMutationKind::AddRow)
        })
        .unwrap()
        .row;
    let attachment = workspace.add_bytes(b"booking".to_vec()).unwrap();
    let file = BridgeMedia {
        name: "Hotel booking.eml".into(),
        media_type: "message/rfc822".into(),
        attachment,
        size: 0,
    };
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: block.clone(),
            row: row.clone(),
            field: field.clone(),
            cell: Some(BridgeCell {
                kind: BridgeCellKind::Files,
                text: None,
                number: 0.0,
                checked: false,
                files: vec![file.clone()],
            }),
            ..mutation(BridgeMutationKind::SetCell)
        })
        .unwrap();
    let rows = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: block.clone(),
            ..query(BridgeQueryKind::Rows)
        })
        .unwrap()
        .rows;
    let cell = rows[0]
        .cells
        .iter()
        .find(|cell| Some(&cell.field) == field.as_ref())
        .unwrap();
    assert_eq!(cell.value.kind, BridgeCellKind::Files);
    assert_eq!(
        cell.value.files,
        [BridgeMedia { size: 7, ..file }],
        "with its size from the attachment"
    );
}
