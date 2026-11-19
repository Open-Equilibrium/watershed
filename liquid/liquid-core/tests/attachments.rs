#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, BlockId, BlockKind, Cell, Created, Error, FieldKind, MAX_ATTACHMENT, MAX_FILES, Match,
    Media, MemoryStore, Mutation, Operation, Origin, PART, PageId, PersonId, Place, Request,
    RowQuery, ViewId,
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
            area,
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

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|at| (at % 251) as u8).collect()
}

fn add(workspace: &mut Workspace, file: &[u8]) -> liquid_core::Attachment {
    let mut upload = workspace.begin_upload(&owner(), file.len() as u64).unwrap();
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
fn the_same_file_has_one_name_and_an_empty_file_has_no_parts() {
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
fn a_media_block_needs_a_finished_attachment() {
    let (mut workspace, page, intro) = page();
    let mut upload = workspace.begin_upload(&owner(), 3).unwrap();
    workspace.upload(&mut upload, b"abc").unwrap();
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
fn an_attachment_is_named_by_its_sha_256_and_different_files_by_different_names() {
    let (mut workspace, ..) = page();
    let abc = add(&mut workspace, b"abc");
    assert_eq!(
        abc.id.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let first = bytes(PART + 3);
    let mut second = first.clone();
    second[PART + 1] ^= 1;
    let (a, b) = (add(&mut workspace, &first), add(&mut workspace, &second));
    assert_ne!(a.id, b.id);
    assert_eq!(read_back(&mut workspace, &a), first);
    assert_eq!(read_back(&mut workspace, &b), second);
}

fn file(name: &str, attachment: &liquid_core::Attachment) -> Media {
    Media {
        name: name.into(),
        media_type: String::new(),
        attachment: attachment.id.clone(),
        size: 0,
    }
}

#[test]
fn a_file_cell_holds_files_of_finished_attachments() {
    let (mut workspace, page, intro) = page();
    let Some(Created::Block { block, .. }) = apply(
        &mut workspace,
        Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: Some(intro),
            name: "Bookings".into(),
        },
    )
    .unwrap() else {
        panic!("a Database Block")
    };
    let Some(Created::Field(files)) = apply(
        &mut workspace,
        Mutation::AddField {
            block: block.clone(),
            place: Place::Last,
            name: "Files".into(),
            kind: FieldKind::File,
        },
    )
    .unwrap() else {
        panic!("a field")
    };
    let Some(Created::Row(row)) = apply(
        &mut workspace,
        Mutation::AddRow {
            block: block.clone(),
            place: Place::Last,
        },
    )
    .unwrap() else {
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
    let all = RowQuery {
        sort: None,
        filter: Some(liquid_core::Filter {
            field: files.clone(),
            matches: Match::NotEmpty,
        }),
        start: 0,
        limit: 10,
        search: None,
    };
    let rows = workspace.rows(&block, &all).unwrap().rows;
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
    missing.attachment = liquid_core::AttachmentId::parse(&"0".repeat(64)).unwrap();
    assert_eq!(
        apply(&mut workspace, set(Cell::Files(vec![missing]))),
        Err(Error::NotFound),
        "only finished attachments"
    );
    let many = vec![file("Same.eml", &booking); MAX_FILES + 1];
    assert_eq!(
        apply(&mut workspace, set(Cell::Files(many))),
        Err(Error::TooMany)
    );
    assert_eq!(
        apply(&mut workspace, set(Cell::Text("Invoice.pdf".into()))),
        Err(Error::WrongKind)
    );
    apply(&mut workspace, set(Cell::Files(Vec::new()))).unwrap();
    let rows = workspace.rows(&block, &all).unwrap();
    assert_eq!(rows.count, 0, "no files is an empty cell");
    let no_files = RowQuery {
        filter: Some(liquid_core::Filter {
            field: files.clone(),
            matches: Match::Is(Cell::Files(Vec::new())),
        }),
        ..all
    };
    assert_eq!(
        workspace.rows(&block, &no_files).unwrap().count,
        1,
        "and so it is as a filter's value"
    );
}
