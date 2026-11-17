//! Attachments and the Media Blocks that show them (ADR-0305): a file's bytes are kept in
//! parts in the store, named by their contents, and a Media Block names its attachment.

#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, AreaId, BlockId, BlockKind, Created, Error, MAX_ATTACHMENT, Media, MemoryStore,
    Mutation, Operation, Origin, PART, PageId, PersonId, Request, ViewId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<Option<Created>, Error> {
    workspace
        .apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        })
        .map(|applied| applied.created)
}

/// A Workspace with one Page holding one Text Block.
fn page() -> (Workspace, PageId, ViewId) {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
    let Some(Created::Area(area)) = apply(
        &mut workspace,
        Mutation::CreateArea {
            name: "Personal".into(),
        },
    )
    .unwrap() else {
        panic!("an area")
    };
    let Some(Created::Page(page)) = apply(
        &mut workspace,
        Mutation::CreatePage {
            area: AreaId::clone(&area),
            parent: None,
            title: "Trip to Lisbon".into(),
        },
    )
    .unwrap() else {
        panic!("a Page")
    };
    let Some(Created::Block { view, .. }) = apply(
        &mut workspace,
        Mutation::AddTextBlock {
            page: page.clone(),
            after: None,
            text: "What we planned to spend:".into(),
        },
    )
    .unwrap() else {
        panic!("a Text Block")
    };
    (workspace, page, view)
}

/// Bytes that differ from part to part, so a misplaced part shows.
fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|at| (at % 251) as u8).collect()
}

fn add(workspace: &mut Workspace, file: &[u8]) -> liquid_core::Attachment {
    let mut upload = workspace.begin_upload(&owner(), file.len() as u64).unwrap();
    // Slices that cut across parts, as a reader hands them over.
    for slice in file.chunks(PART / 3 + 7) {
        workspace.upload(&mut upload, slice).unwrap();
    }
    assert_eq!(upload.written(), file.len() as u64);
    workspace.finish_upload(upload).unwrap()
}

fn read_back(workspace: &mut Workspace, attachment: &liquid_core::Attachment) -> Vec<u8> {
    let mut read = Vec::new();
    let mut index = 0;
    while let Some(part) = workspace.attachment_part(&attachment.id, index).unwrap() {
        assert!(part.len() <= PART, "parts of at most 1 MiB");
        read.extend(part);
        index += 1;
    }
    read
}

#[test]
fn a_file_is_kept_in_parts_and_a_media_block_shows_it() {
    let (mut workspace, page, intro) = page();
    let file = bytes(2 * PART + PART / 2);
    let attachment = add(&mut workspace, &file);
    assert_eq!(attachment.size, file.len() as u64);
    assert_eq!(
        attachment.id.as_str().len(),
        64,
        "named by its SHA-256, in hex"
    );
    assert_eq!(read_back(&mut workspace, &attachment), file);
    assert_eq!(
        workspace.attachment_size(&attachment.id).unwrap(),
        Some(file.len() as u64)
    );

    let added = apply(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: Some(intro),
            name: "Lisbon trip budget.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.id.clone(),
        },
    )
    .unwrap();
    let Some(Created::Block { block, view }) = added else {
        panic!("a Media Block")
    };
    let kinds: Vec<_> = workspace
        .views(&page, 0, 10)
        .unwrap()
        .into_iter()
        .map(|view| view.kind)
        .collect();
    assert_eq!(kinds, [BlockKind::Text, BlockKind::Media]);
    workspace.close_page(&page);
    assert_eq!(
        workspace.media(&page, &block).unwrap(),
        Some(Media {
            name: "Lisbon trip budget.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.id.clone(),
            size: file.len() as u64,
        }),
        "stored, so read again after the Page closed"
    );
    assert_eq!(
        workspace.text(&page, &block).unwrap(),
        None,
        "no Text Block"
    );
    let recorded = workspace.history(None, 1).unwrap();
    assert!(matches!(
        &recorded[0].operation,
        Operation::Mutation(Mutation::AddMediaBlock { .. })
    ));

    apply(
        &mut workspace,
        Mutation::RemoveView {
            page: page.clone(),
            view,
        },
    )
    .unwrap();
    assert_eq!(workspace.media(&page, &block).unwrap(), None, "deleted");
}

#[test]
fn the_same_file_added_twice_has_one_name_and_an_empty_file_none_of_its_parts() {
    let (mut workspace, ..) = page();
    let file = bytes(PART + 1);
    let first = add(&mut workspace, &file);
    let second = add(&mut workspace, &file);
    assert_eq!(first, second);
    assert_eq!(read_back(&mut workspace, &second), file);

    let empty = add(&mut workspace, &[]);
    assert_eq!(empty.size, 0);
    assert_eq!(read_back(&mut workspace, &empty), Vec::<u8>::new());
}

#[test]
fn a_file_past_5_gib_or_past_its_declared_size_is_refused() {
    let (mut workspace, ..) = page();
    assert_eq!(MAX_ATTACHMENT, 5 << 30);
    assert!(matches!(
        workspace.begin_upload(&owner(), MAX_ATTACHMENT + 1),
        Err(Error::TooLarge)
    ));
    assert!(workspace.begin_upload(&owner(), MAX_ATTACHMENT).is_ok());

    // A file that grows while it is read is refused, so the cap holds.
    let mut upload = workspace.begin_upload(&owner(), 4).unwrap();
    assert_eq!(
        workspace.upload(&mut upload, b"12345"),
        Err(Error::TooLarge)
    );
    // One that shrinks is refused at the end.
    let mut upload = workspace.begin_upload(&owner(), 4).unwrap();
    workspace.upload(&mut upload, b"123").unwrap();
    assert_eq!(
        workspace.finish_upload(upload).err(),
        Some(Error::OutOfRange)
    );

    let stranger = Actor::Person(PersonId::new("someone else"));
    assert!(matches!(
        workspace.begin_upload(&stranger, 4),
        Err(Error::NotAllowed)
    ));
}

#[test]
fn a_media_block_needs_a_finished_attachment() {
    let (mut workspace, page, intro) = page();
    let mut upload = workspace.begin_upload(&owner(), 3).unwrap();
    workspace.upload(&mut upload, b"abc").unwrap();
    // The name the finished upload would get, which no finished attachment has yet.
    let unfinished = liquid_core::AttachmentId::parse(
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )
    .unwrap();
    let before = workspace.history(None, usize::MAX).unwrap().len();
    let refused = apply(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: Some(intro.clone()),
            name: "abc.txt".into(),
            media_type: "text/plain".into(),
            attachment: unfinished.clone(),
        },
    );
    assert_eq!(refused, Err(Error::NotFound));
    assert_eq!(workspace.history(None, usize::MAX).unwrap().len(), before);
    assert_eq!(workspace.views(&page, 0, 10).unwrap().len(), 1);

    workspace.discard_upload(upload).unwrap();
    assert_eq!(workspace.attachment_size(&unfinished).unwrap(), None);
    assert_eq!(liquid_core::AttachmentId::parse("not a hash"), None);
}

#[test]
fn a_media_block_takes_no_text_writes() {
    let (mut workspace, page, intro) = page();
    let attachment = add(&mut workspace, b"GIF89a");
    let Some(Created::Block { block, .. }) = apply(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: Some(intro),
            name: "tram.gif".into(),
            media_type: "image/gif".into(),
            attachment: attachment.id,
        },
    )
    .unwrap() else {
        panic!("a Media Block")
    };
    let edit = Mutation::EditText {
        page: page.clone(),
        block: BlockId::clone(&block),
        at: 0,
        delete: 0,
        insert: "x".into(),
    };
    assert_eq!(apply(&mut workspace, edit), Err(Error::NotFound));
    let code = Mutation::TurnIntoCode { page, block };
    assert_eq!(apply(&mut workspace, code), Err(Error::NotFound));
}

#[test]
fn a_large_file_comes_back_whole() {
    let (mut workspace, ..) = page();
    let file = bytes(33 * PART + 5);
    let attachment = add(&mut workspace, &file);
    assert_eq!(read_back(&mut workspace, &attachment), file);
}

#[test]
fn an_upload_finishes_only_in_the_workspace_that_began_it() {
    let (mut first, ..) = page();
    let (mut second, ..) = page();
    let mut upload = first.begin_upload(&owner(), 3).unwrap();
    first.upload(&mut upload, b"abc").unwrap();
    // The parts are in the first Workspace's store, so the second has nothing to name.
    assert!(matches!(
        second.finish_upload(upload),
        Err(Error::Storage(_))
    ));
}
