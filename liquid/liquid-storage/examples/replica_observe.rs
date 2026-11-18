use std::path::Path;
use std::time::{Duration, Instant};

use liquid_core::{
    Actor, BlockId, Content, Created, Element, Frame, InkColor, MAX_BOARD_POINTS, MAX_POINTS,
    Mutation, Origin, PageId, Pen, PersonId, Request, Workspace,
};
use liquid_storage::Sqlite;

const KEYSTROKES: usize = 2_000;
const ATTACHMENT: usize = 256 << 20;
const BLOCKS: usize = 1_000;
const STROKES: usize = 2_000;
const STROKE_POINTS: usize = 150;
const KEY: [u8; 32] = [7; 32];

fn request(mutation: Mutation) -> Request {
    Request {
        actor: Actor::Person(PersonId::new("observer")),
        origin: Origin::Ui,
        mutation,
    }
}

fn created(workspace: &mut Workspace<Sqlite>, mutation: Mutation) -> Created {
    workspace
        .apply(request(mutation))
        .expect("the fixture mutation is accepted")
        .created
        .expect("a created object")
}

fn page(workspace: &mut Workspace<Sqlite>) -> PageId {
    let Created::Area(area) = created(
        workspace,
        Mutation::CreateArea {
            name: "Trips".into(),
        },
    ) else {
        panic!("an area")
    };
    let create = Mutation::CreatePage {
        area,
        parent: None,
        title: "Lisbon".into(),
    };
    match created(workspace, create) {
        Created::Page(page) => page,
        _ => panic!("a Page"),
    }
}

fn summary(name: &str, mut times: Vec<Duration>) {
    times.sort();
    let at = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    println!(
        "{name}: median {:.1} µs, p99 {:.1} µs, max {:.1} µs",
        at(0.5).as_secs_f64() * 1e6,
        at(0.99).as_secs_f64() * 1e6,
        times.last().expect("a summary has samples").as_secs_f64() * 1e6
    );
}

fn main() {
    if let Some(argument) = std::env::args().nth(1) {
        if argument == "--attachment" {
            attachment();
        } else if argument == "--whiteboard" {
            whiteboard();
        } else {
            reopen(Path::new(&argument));
        }
        return;
    }
    let dir = std::env::temp_dir().join(format!("liquid-replica-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the temporary directory can be created");
    let path = dir.join("replica.db");

    let mut workspace = Workspace::create(
        PersonId::new("observer"),
        Sqlite::open(&path, &KEY).expect("a new replica opens"),
    )
    .expect("the replica accepts a new Workspace");
    let lisbon = page(&mut workspace);
    let add = Mutation::AddTextBlock {
        page: lisbon.clone(),
        after: None,
        text: String::new(),
    };
    let Created::Block { block, .. } = created(&mut workspace, add) else {
        panic!("a Block")
    };
    let mut times = Vec::with_capacity(KEYSTROKES);
    for i in 0..KEYSTROKES {
        let edit = Mutation::EditText {
            page: lisbon.clone(),
            block: block.clone(),
            at: i,
            delete: 0,
            insert: if i % 7 == 6 { " " } else { "x" }.into(),
        };
        let start = Instant::now();
        workspace
            .apply(request(edit))
            .expect("the fixture edit is accepted");
        times.push(start.elapsed());
    }
    summary(
        &format!("typing, {KEYSTROKES} keystrokes, one durable commit each, per keystroke"),
        times,
    );

    let mut after = None;
    for _ in 0..BLOCKS {
        let add = Mutation::AddTextBlock {
            page: lisbon.clone(),
            after: after.clone(),
            text: "Five days in Lisbon, one in Porto.".into(),
        };
        if let Created::Block { view, .. } = created(&mut workspace, add) {
            after = Some(view);
        }
    }
    drop(workspace);
    let size: u64 = std::fs::read_dir(&dir)
        .expect("the replica directory is readable")
        .map(|entry| {
            entry
                .expect("a directory entry is readable")
                .metadata()
                .expect("its metadata is readable")
                .len()
        })
        .sum();
    println!(
        "database with its log after both: {:.1} MiB",
        size as f64 / 1_048_576.0
    );
    println!("replica: {}", path.display());
}

fn reopen(path: &Path) {
    let start = Instant::now();
    let mut workspace =
        Workspace::open(Sqlite::open(path, &KEY).expect("the replica opens with its key"))
            .expect("the replica reopens as a Workspace");
    let area = workspace
        .areas()
        .expect("the replica lists its Areas")
        .remove(0)
        .id;
    let page = workspace
        .pages(&area)
        .expect("the replica lists its Pages")
        .remove(0)
        .id;
    let views = workspace
        .views(&page, 0, usize::MAX)
        .expect("the replica reads the Page's Views");
    println!(
        "a new process opening the replica and reading that Page's {} Views: {:.1} ms",
        views.len(),
        start.elapsed().as_secs_f64() * 1e3
    );
    drop(workspace);
    let name = path.file_name().expect("the replica has a file name");
    for suffix in ["", "-wal", "-shm"] {
        let mut file = name.to_os_string();
        file.push(suffix);
        match std::fs::remove_file(path.with_file_name(&file)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("the replica file {file:?} is removed: {error}"),
        }
    }
    let _ = std::fs::remove_dir(path.parent().expect("the replica is in its own directory"));
}

fn attachment() {
    let dir =
        std::env::temp_dir().join(format!("liquid-attachment-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the temporary directory can be created");
    let path = dir.join("replica.db");
    let mut workspace = Workspace::create(
        PersonId::new("observer"),
        Sqlite::open(&path, &KEY).expect("a new replica opens"),
    )
    .expect("the replica accepts a new Workspace");
    let part: Vec<u8> = (0..liquid_core::PART).map(|at| (at % 251) as u8).collect();
    let owner = Actor::Person(PersonId::new("observer"));
    let start = Instant::now();
    let mut upload = workspace
        .begin_upload(&owner, ATTACHMENT as u64)
        .expect("the file is within the cap");
    let mut times = Vec::new();
    for _ in 0..ATTACHMENT / liquid_core::PART {
        let started = Instant::now();
        workspace
            .upload(&mut upload, &part)
            .expect("the store keeps the part");
        times.push(started.elapsed());
    }
    let attachment = workspace
        .finish_upload(upload)
        .expect("the upload finishes");
    let added = start.elapsed();
    summary("adding a 256 MiB file, per 1 MiB part", times);
    println!(
        "adding it whole, with its SHA-256: {:.2} s, {:.0} MiB/s",
        added.as_secs_f64(),
        256.0 / added.as_secs_f64()
    );
    let start = Instant::now();
    let mut index = 0;
    while workspace
        .attachment_part(&attachment.id, index)
        .expect("the store reads the part")
        .is_some()
    {
        index += 1;
    }
    let read = start.elapsed();
    println!(
        "reading it back part by part: {:.2} s, {:.0} MiB/s",
        read.as_secs_f64(),
        256.0 / read.as_secs_f64()
    );
    drop(workspace);
    std::fs::remove_dir_all(&dir).expect("the replica is removed");
}

fn stroke(row: usize, points: usize) -> Element {
    let y = (row % 400) as f32 * 3.0;
    let bytes: Vec<u8> = (0..points)
        .flat_map(|at| [at as f32 * 0.5, y + (at % 7) as f32, 0.5])
        .flat_map(f32::to_le_bytes)
        .collect();
    Element {
        frame: Frame {
            x: 0.0,
            y: f64::from(y),
            width: points as f64 * 0.5,
            height: 6.0,
            rotation: 0.0,
        },
        color: InkColor::Text,
        width: 1,
        doc_page: 0,
        content: Content::Stroke {
            pen: Pen::Fountain,
            points: bytes,
        },
    }
}

fn draw(workspace: &mut Workspace<Sqlite>, page: &PageId, block: &BlockId, add: Vec<Element>) {
    workspace
        .apply(request(Mutation::EditDrawing {
            page: page.clone(),
            block: block.clone(),
            add,
            change: Vec::new(),
            remove: Vec::new(),
        }))
        .expect("the board takes the strokes");
}

fn whiteboard() {
    let dir =
        std::env::temp_dir().join(format!("liquid-whiteboard-observe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the temporary directory can be created");
    let path = dir.join("replica.db");
    let mut workspace = Workspace::create(
        PersonId::new("observer"),
        Sqlite::open(&path, &KEY).expect("a new replica opens"),
    )
    .expect("the replica accepts a new Workspace");
    let page = page(&mut workspace);
    let add = |workspace: &mut Workspace<Sqlite>| {
        let added = Mutation::AddWhiteboardBlock {
            page: page.clone(),
            after: None,
        };
        match created(workspace, added) {
            Created::Block { block, .. } => block,
            _ => panic!("a Whiteboard"),
        }
    };
    let block = add(&mut workspace);
    let mut times = Vec::new();
    for row in 0..STROKES {
        let started = Instant::now();
        draw(
            &mut workspace,
            &page,
            &block,
            vec![stroke(row, STROKE_POINTS)],
        );
        times.push(started.elapsed());
    }
    summary(
        &format!("drawing {STROKES} strokes of {STROKE_POINTS} points, per stroke"),
        times,
    );
    let read = |workspace: &mut Workspace<Sqlite>, block: &BlockId| {
        workspace.close_page(&page);
        let start = Instant::now();
        let board = workspace
            .drawing(&page, block, 0..1)
            .expect("the board reads")
            .expect("a live board");
        (start.elapsed(), board.elements.len())
    };
    let (took, elements) = read(&mut workspace, &block);
    println!(
        "loading the Page and reading its {elements} strokes: {:.1} ms",
        took.as_secs_f64() * 1e3
    );
    let full = add(&mut workspace);
    let longest = MAX_BOARD_POINTS / MAX_POINTS;
    for row in 0..longest {
        draw(&mut workspace, &page, &full, vec![stroke(row, MAX_POINTS)]);
    }
    let (took, elements) = read(&mut workspace, &full);
    println!(
        "loading the Page and reading a board at its cap, {elements} strokes of {MAX_POINTS} points: {:.1} ms",
        took.as_secs_f64() * 1e3
    );
    drop(workspace);
    std::fs::remove_dir_all(&dir).expect("the replica is removed");
}
