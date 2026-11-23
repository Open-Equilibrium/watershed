#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Actor, Attachment, AttachmentId, BlockId, BlockKind, Cell, Created, Error, FieldKind, Filter,
    Layout, MAX_ATTACHMENT, MAX_FILES, Match, Media, Mutation, Operation, PART, PageId, PersonId,
    Place, RowQuery, ViewId,
};

fn page() -> (Workspace, PageId, ViewId) {
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
    let Created::Block { view, .. } = created(
        &mut workspace,
        Mutation::AddTextBlock {
            page: page.clone(),
            after: None,
            text: "What we planned to spend:".into(),
        },
    ) else {
        panic!("a Text Block")
    };
    (workspace, page, view)
}

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|at| (at % 251) as u8).collect()
}

fn add(workspace: &mut Workspace, file: &[u8]) -> Attachment {
    let mut upload = workspace.begin_upload(&owner(), file.len() as u64).unwrap();
    for slice in file.chunks(PART / 3 + 7) {
        workspace.upload(&mut upload, slice).unwrap();
    }
    assert_eq!(upload.written(), file.len() as u64);
    workspace.finish_upload(upload).unwrap()
}

fn read_back(workspace: &mut Workspace, attachment: &Attachment) -> Vec<u8> {
    let mut read = Vec::new();
    let mut index = 0;
    while let Some(part) = workspace.attachment_part(&attachment.id, index).unwrap() {
        assert!(part.len() <= PART, "parts of at most 1 MiB");
        read.extend(part);
        index += 1;
    }
    read
}

fn file(name: &str, attachment: &Attachment) -> Media {
    Media {
        name: name.into(),
        media_type: String::new(),
        attachment: attachment.id.clone(),
        size: 0,
    }
}

#[test]
fn a_file_is_kept_in_parts_and_a_media_block_shows_it() {
    let (mut workspace, page, intro) = page();
    let content = bytes(2 * PART + PART / 2);
    let attachment = add(&mut workspace, &content);
    assert_eq!(attachment.size, content.len() as u64);
    assert_eq!(
        attachment.id.as_str().len(),
        64,
        "named by its SHA-256, in hex"
    );
    assert_eq!(read_back(&mut workspace, &attachment), content);
    assert_eq!(
        workspace.attachment_size(&attachment.id).unwrap(),
        Some(content.len() as u64)
    );

    let Created::Block { block, view } = created(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: Some(intro),
            name: "Lisbon trip budget.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.id.clone(),
        },
    ) else {
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
            size: content.len() as u64,
        }),
        "stored, so read again after the Page closed"
    );
    assert_eq!(
        workspace.text(&page, &block).unwrap(),
        None,
        "no Text Block"
    );
    assert!(matches!(
        &workspace.history(None, 1).unwrap()[0].operation,
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
fn attachments_are_named_by_their_sha_256_and_the_same_file_has_one_name() {
    let (mut workspace, ..) = page();
    let abc = add(&mut workspace, b"abc");
    assert_eq!(
        abc.id.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let first = bytes(PART + 3);
    let mut second = first.clone();
    second[PART + 1] ^= 1;
    let (a, again) = (add(&mut workspace, &first), add(&mut workspace, &first));
    let b = add(&mut workspace, &second);
    assert_eq!(a, again);
    assert_ne!(a.id, b.id);
    assert_eq!(read_back(&mut workspace, &again), first);
    assert_eq!(read_back(&mut workspace, &b), second);

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

    let mut upload = workspace.begin_upload(&owner(), 4).unwrap();
    assert_eq!(workspace.upload(&mut upload, b"12345"), Err(Error::Changed));
    let mut upload = workspace.begin_upload(&owner(), 4).unwrap();
    workspace.upload(&mut upload, b"123").unwrap();
    assert_eq!(workspace.finish_upload(upload).err(), Some(Error::Changed));

    let stranger = Actor::Person(PersonId::new("someone else"));
    assert!(matches!(
        workspace.begin_upload(&stranger, 4),
        Err(Error::NotAllowed)
    ));
}

#[test]
fn a_media_block_needs_a_finished_attachment_and_takes_no_text_writes() {
    let (mut workspace, page, intro) = page();
    let mut upload = workspace.begin_upload(&owner(), 3).unwrap();
    workspace.upload(&mut upload, b"abc").unwrap();
    let unfinished =
        AttachmentId::parse("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
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
    assert_eq!(refused.unwrap_err(), Error::NotFound);
    assert_eq!(workspace.history(None, usize::MAX).unwrap().len(), before);
    assert_eq!(workspace.views(&page, 0, 10).unwrap().len(), 1);
    workspace.discard_upload(upload).unwrap();
    assert_eq!(workspace.attachment_size(&unfinished).unwrap(), None);
    assert_eq!(AttachmentId::parse("not a hash"), None);

    let attachment = add(&mut workspace, b"GIF89a");
    let Created::Block { block, .. } = created(
        &mut workspace,
        Mutation::AddMediaBlock {
            page: page.clone(),
            after: Some(intro),
            name: "tram.gif".into(),
            media_type: "image/gif".into(),
            attachment: attachment.id,
        },
    ) else {
        panic!("a Media Block")
    };
    let edit = Mutation::EditText {
        page: page.clone(),
        block: BlockId::clone(&block),
        at: 0,
        delete: 0,
        insert: "x".into(),
    };
    assert_eq!(apply(&mut workspace, edit).unwrap_err(), Error::NotFound);
    let code = Mutation::TurnIntoCode { page, block };
    assert_eq!(apply(&mut workspace, code).unwrap_err(), Error::NotFound);
}

#[test]
fn an_upload_finishes_only_in_the_workspace_that_began_it() {
    let (mut first, ..) = page();
    let (mut second, ..) = page();
    let mut upload = first.begin_upload(&owner(), 3).unwrap();
    first.upload(&mut upload, b"abc").unwrap();
    assert!(matches!(
        second.finish_upload(upload),
        Err(Error::Storage(_))
    ));
}

#[test]
fn a_file_cell_holds_files_of_finished_attachments() {
    let (mut workspace, page, intro) = page();
    let Created::Block { block, .. } = created(
        &mut workspace,
        Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: Some(intro),
            name: "Bookings".into(),
            layout: Layout::Table,
        },
    ) else {
        panic!("a Database Block")
    };
    let Created::Field(files) = created(
        &mut workspace,
        Mutation::AddField {
            block: block.clone(),
            place: Place::Last,
            name: "Files".into(),
            kind: FieldKind::File,
        },
    ) else {
        panic!("a field")
    };
    let Created::Row(row) = created(
        &mut workspace,
        Mutation::AddRow {
            block: block.clone(),
            place: Place::Last,
        },
    ) else {
        panic!("a row")
    };
    let set = |value: Cell| Mutation::SetCell {
        block: block.clone(),
        row: row.clone(),
        field: files.clone(),
        value,
    };
    let booking = add(&mut workspace, b"booking");
    let invoice = add(&mut workspace, &bytes(PART + 1));
    let both = vec![
        file("Hotel booking.eml", &booking),
        file("Invoice.pdf", &invoice),
    ];
    apply(&mut workspace, set(Cell::Files(both))).unwrap();
    workspace.close_page(&page);
    let stored = RowQuery {
        filter: Some(Filter {
            field: files.clone(),
            matches: Match::NotEmpty,
        }),
        limit: 10,
        ..all()
    };
    let rows = workspace.rows(&block, &stored).unwrap().rows;
    let Some(Cell::Files(read)) = rows[0].cell(&files) else {
        panic!("files, stored")
    };
    assert_eq!(
        read.iter()
            .map(|file| (file.name.as_str(), &file.attachment, file.size))
            .collect::<Vec<_>>(),
        [
            ("Hotel booking.eml", &booking.id, 7),
            ("Invoice.pdf", &invoice.id, PART as u64 + 1)
        ],
        "in their order, each its attachment with the size the store keeps"
    );

    let mut missing = file("Lost.pdf", &booking);
    missing.attachment = AttachmentId::parse(&"0".repeat(64)).unwrap();
    assert_eq!(
        apply(&mut workspace, set(Cell::Files(vec![missing]))).unwrap_err(),
        Error::NotFound,
        "only finished attachments"
    );
    let many = vec![file("Same.eml", &booking); MAX_FILES + 1];
    assert_eq!(
        apply(&mut workspace, set(Cell::Files(many))).unwrap_err(),
        Error::TooMany
    );
    assert_eq!(
        apply(&mut workspace, set(Cell::Text("Invoice.pdf".into()))).unwrap_err(),
        Error::WrongKind
    );
    workspace.close_page(&page);
    assert_eq!(
        workspace.rows(&block, &stored).unwrap().rows[0].cell(&files),
        Some(&Cell::Files(read.clone())),
        "a refused write leaves the files as they were"
    );
    apply(&mut workspace, set(Cell::Files(Vec::new()))).unwrap();
    assert_eq!(
        workspace.rows(&block, &stored).unwrap().count,
        0,
        "no files is an empty cell"
    );
    let no_files = RowQuery {
        filter: Some(Filter {
            field: files.clone(),
            matches: Match::Is(Cell::Files(Vec::new())),
        }),
        ..all()
    };
    assert_eq!(
        workspace.rows(&block, &no_files).unwrap().count,
        1,
        "and so it is as a filter's value"
    );
}
