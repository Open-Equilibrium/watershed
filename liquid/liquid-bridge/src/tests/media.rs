use crate::api::media::BridgeMedia;
use crate::api::workspace::{
    BridgeBlockKind, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind,
};

use super::workspace::{lisbon, mutation, open, query, views};

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|at| (at % 253) as u8).collect()
}

#[test]
fn a_file_goes_in_from_its_path_shows_in_a_media_block_and_saves_out_unchanged() {
    let (folder, workspace) = open("media");
    let (_, page, _, intro) = lisbon(&workspace);
    let file = bytes(2 * liquid_core::PART + 12_345);
    let path = folder.0.join("Lisbon trip budget.pdf");
    std::fs::write(&path, &file).unwrap();

    let mut events = Vec::new();
    workspace
        .add_file_with(path.to_string_lossy().into(), |upload| {
            events.push(upload);
            true
        })
        .unwrap();
    let last = events.last().unwrap().clone();
    let attachment = last.attachment.clone().expect("the last event names it");
    assert_eq!(
        (last.written, last.size),
        (file.len() as i64, file.len() as i64)
    );
    assert!(events.len() > 2, "progress while the parts go in");
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].written <= pair[1].written)
    );

    let added = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro),
            text: Some("Lisbon trip budget.pdf".into()),
            media_type: Some("application/pdf".into()),
            attachment: Some(attachment.clone()),
            ..mutation(BridgeMutationKind::AddMediaBlock)
        })
        .unwrap();
    assert_eq!(views(&workspace, &page)[1].kind, BridgeBlockKind::Media);
    let media = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: added.block.clone(),
            ..query(BridgeQueryKind::Media)
        })
        .unwrap()
        .media;
    assert_eq!(
        media,
        Some(BridgeMedia {
            name: "Lisbon trip budget.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.clone(),
            size: file.len() as i64,
        })
    );

    let saved = folder.0.join("saved.pdf");
    let neighbor = folder.0.join(".saved.pdf.liquid-saving");
    std::fs::write(&neighbor, b"not Liquid's").unwrap();
    workspace
        .save_attachment(attachment.clone(), saved.to_string_lossy().into())
        .unwrap();
    assert_eq!(std::fs::read(&saved).unwrap(), file);
    assert_eq!(std::fs::read(&neighbor).unwrap(), b"not Liquid's");
    std::fs::remove_file(&neighbor).unwrap();
    let names: Vec<_> = std::fs::read_dir(&folder.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(
        names
            .iter()
            .all(|name| !name.to_string_lossy().starts_with(".liquid-saving-")),
        "no partial copy stays beside it: {names:?}"
    );
    let long = folder.0.join(format!("{}.pdf", "b".repeat(251)));
    workspace
        .save_attachment(attachment.clone(), long.to_string_lossy().into())
        .unwrap();
    assert_eq!(std::fs::read(&long).unwrap(), file);
    assert_eq!(workspace.read_attachment(attachment).unwrap(), file);
}

#[test]
fn a_missing_file_and_an_unknown_attachment_are_refused_and_nothing_is_written() {
    let (folder, workspace) = open("missing-file");
    let missing = folder.0.join("gone.png").to_string_lossy().into();
    assert!(workspace.add_file_with(missing, |_| true).is_err());
    let unknown = "0".repeat(64);
    assert!(workspace.read_attachment(unknown.clone()).is_err());
    assert!(workspace.read_attachment("not a name".into()).is_err());
    let target = folder.0.join("never.png");
    assert!(
        workspace
            .save_attachment(unknown, target.to_string_lossy().into())
            .is_err()
    );
    assert!(!target.exists(), "no file for an unknown attachment");
}
