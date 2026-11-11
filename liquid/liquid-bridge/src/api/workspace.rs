//! The Liquid Workspace for the app. It crosses the bridge only through four message-shaped
//! calls: apply a mutation, query, subscribe to a resource's changes and close a subscription
//! (client architecture rule 2, ADR-0216). Each runs on flutter_rust_bridge's worker threads,
//! never on the Dart main isolate.
//!
//! Messages are flat structs whose kind names the parts they fill, so Dart needs no code
//! generation for enums with data. Identities cross as text. A change carries only its
//! resource and that resource's sequence number; the reader then queries the bounded window
//! it shows (rule 3).

use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use liquid_core::{
    Actor, AreaId, BlockId, BlockKind, Changed, Created, Error, FieldId, Mutation, Origin, PageId,
    PersonId, Place, Request, RowId, TextStyle, ViewId, Workspace,
};
use liquid_storage::Sqlite;

use super::database::{
    BridgeCell, BridgeDatabase, BridgeFieldKind, BridgeFilter, BridgeRow, BridgeSort, row_query,
};

use crate::frb_generated::StreamSink;
use crate::keys::{Keychain, Keys, random_hex, replica_key};

/// The most items one query answers, so no answer carries an unbounded list.
pub const MAX_WINDOW: i64 = 500;

/// What a [`BridgeMutation`] does; each kind names the parts it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMutationKind {
    /// Adds an area named `text`.
    CreateArea,
    /// Adds a Page titled `text` to `area`, under `parent` if given.
    CreatePage,
    /// Retitles `page` to `text`.
    RenamePage,
    /// Adds a Text Block holding `text` to `page`, after the View `after` or first.
    AddTextBlock,
    /// Replaces `delete` units at `at` in `block` on its home `page` with `text`; positions
    /// count UTF-16 units, as Dart does.
    EditText,
    /// Removes `view` from `page`.
    RemoveView,
    /// Ends `block`, shown by `view` on its home `page`, at `at`, as ⏎ does: the text from
    /// there moves to a new Text Block of `style` after `view`.
    SplitTextBlock,
    /// Appends the Text Block of `view` on `page` to the one above, as ⌫ at its start does.
    JoinTextBlock,
    /// Sets `block`'s `style` on its home `page`.
    SetTextStyle,
    /// Starts a Section at `view` on `page`, or stops one, as `boundary` says.
    SetSectionBoundary,
    /// Adds a Database Block named `text`, with a text field "Name", to `page`, after the
    /// View `after` or first.
    AddDatabaseBlock,
    /// Renames the Database `block` on its home `page` to `text`.
    RenameDatabase,
    /// Adds a field named `text` of `field_kind` to the Database `block` on its home `page`:
    /// last, after the field `after`, or first.
    AddField,
    /// Renames `field` of the Database `block` on its home `page` to `text`.
    RenameField,
    /// Removes `field` from the Database `block` on its home `page`.
    RemoveField,
    /// Adds an option named `text` of `color` to the select `field` of the Database `block`
    /// on its home `page`.
    AddOption,
    /// Adds an empty row to the Database `block` on its home `page`: last, after the row
    /// `after`, or first.
    AddRow,
    /// Removes `row` from the Database `block` on its home `page`.
    RemoveRow,
    /// Sets `row`'s cell of `field` in the Database `block` on its home `page` to `cell`.
    SetCell,
}

/// How a Text Block reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTextStyle {
    /// A paragraph.
    Paragraph,
    /// A heading of level 1.
    Heading1,
    /// A heading of level 2.
    Heading2,
    /// A heading of level 3.
    Heading3,
    /// A bulleted list item.
    Bullet,
    /// A numbered list item.
    Numbered,
    /// A to-do not yet done.
    Todo,
    /// A done to-do.
    Done,
    /// A quote.
    Quote,
}

impl BridgeTextStyle {
    fn to_core(self) -> TextStyle {
        match self {
            Self::Paragraph => TextStyle::Paragraph,
            Self::Heading1 => TextStyle::Heading(1),
            Self::Heading2 => TextStyle::Heading(2),
            Self::Heading3 => TextStyle::Heading(3),
            Self::Bullet => TextStyle::Bullet,
            Self::Numbered => TextStyle::Numbered,
            Self::Todo => TextStyle::Todo { done: false },
            Self::Done => TextStyle::Todo { done: true },
            Self::Quote => TextStyle::Quote,
        }
    }

    fn of(style: TextStyle) -> Self {
        match style {
            TextStyle::Heading(1) => Self::Heading1,
            TextStyle::Heading(2) => Self::Heading2,
            TextStyle::Heading(_) => Self::Heading3,
            TextStyle::Bullet => Self::Bullet,
            TextStyle::Numbered => Self::Numbered,
            TextStyle::Todo { done: false } => Self::Todo,
            TextStyle::Todo { done: true } => Self::Done,
            TextStyle::Quote => Self::Quote,
            TextStyle::Paragraph => Self::Paragraph,
        }
    }
}

/// One write, as Dart sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeMutation {
    /// What the write does.
    pub kind: BridgeMutationKind,
    /// An area.
    pub area: Option<String>,
    /// A Page.
    pub page: Option<String>,
    /// The Page a new Page nests under.
    pub parent: Option<String>,
    /// A Block.
    pub block: Option<String>,
    /// A View.
    pub view: Option<String>,
    /// The View, field or row a new one follows.
    pub after: Option<String>,
    /// Whether a new field or row goes after all others as the core applies it, rather
    /// than first or after `after` (ADR-0247).
    pub last: bool,
    /// A name, title or text.
    pub text: Option<String>,
    /// Where an edit starts.
    pub at: i64,
    /// How many units an edit removes.
    pub delete: i64,
    /// A Text Block's style.
    pub style: BridgeTextStyle,
    /// Whether a Section starts at `view`.
    pub boundary: bool,
    /// A Database field.
    pub field: Option<String>,
    /// A Database row.
    pub row: Option<String>,
    /// A new field's kind.
    pub field_kind: Option<BridgeFieldKind>,
    /// A new option's color, an index into the tag colors.
    pub color: i64,
    /// A cell's new value.
    pub cell: Option<BridgeCell>,
}

/// What an applied write recorded and created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApplied {
    /// The Action that recorded it.
    pub action: String,
    /// A new area.
    pub area: Option<String>,
    /// A new Page.
    pub page: Option<String>,
    /// A new Block.
    pub block: Option<String>,
    /// The new Block's View.
    pub view: Option<String>,
    /// A new Database field.
    pub field: Option<String>,
    /// A new Database row.
    pub row: Option<String>,
    /// A new option of a select field.
    pub option: Option<String>,
}

/// What a [`BridgeQuery`] reads; each kind names the parts it uses and fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeQueryKind {
    /// A window of the areas into `areas`, and their number into `count`.
    Areas,
    /// A window of `area`'s Page tree into `pages`, and its length into `count`.
    Pages,
    /// A window of `page`'s Views into `views`, and their number into `count`.
    Views,
    /// `block`'s text and style on its home `page` into `text` and `style`; none once the
    /// Block is removed.
    Text,
    /// The Database `block`'s name and fields on its home `page` into `database`; none
    /// once the Block is removed or if it is no Database.
    Database,
    /// A window of the Database `block`'s rows on its home `page` into `rows`, kept by
    /// `filter` and ordered by `sort`, and the number of rows `filter` keeps into `count`.
    Rows,
}

/// One read, as Dart sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeQuery {
    /// What it reads.
    pub kind: BridgeQueryKind,
    /// An area.
    pub area: Option<String>,
    /// A Page.
    pub page: Option<String>,
    /// A Block.
    pub block: Option<String>,
    /// Where a window starts.
    pub start: i64,
    /// How many items a window takes, at most [`MAX_WINDOW`].
    pub limit: i64,
    /// How a row window is ordered; the rows' own order without one.
    pub sort: Option<BridgeSort>,
    /// Which rows a row window keeps; all without one.
    pub filter: Option<BridgeFilter>,
}

/// An area in the Explorer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArea {
    /// Its identity.
    pub id: String,
    /// Its name.
    pub name: String,
}

/// A Page in an area's tree, which lists each Page before its descendants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePage {
    /// Its identity.
    pub id: String,
    /// The Page it nests under.
    pub parent: Option<String>,
    /// Its title.
    pub title: String,
}

/// The kind of Block a View shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeBlockKind {
    /// A Text Block.
    Text,
    /// A Database Block.
    Database,
    /// A Block of a kind a later version adds, which shows as a placeholder (ADR-0245).
    Unknown,
}

/// A View on a Page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeView {
    /// Its identity.
    pub id: String,
    /// The Block it shows.
    pub block: String,
    /// Whether a Section starts here.
    pub boundary: bool,
    /// The kind of Block it shows.
    pub kind: BridgeBlockKind,
}

/// The answer to a query; the query's kind names the parts it fills.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAnswer {
    /// Areas.
    pub areas: Vec<BridgeArea>,
    /// Pages.
    pub pages: Vec<BridgePage>,
    /// Views.
    pub views: Vec<BridgeView>,
    /// How many items the whole list holds.
    pub count: i64,
    /// A Block's text.
    pub text: Option<String>,
    /// A Text Block's style, with its text.
    pub style: Option<BridgeTextStyle>,
    /// A Database's name and fields.
    pub database: Option<BridgeDatabase>,
    /// A Database's rows.
    pub rows: Vec<BridgeRow>,
}

/// What kind of resource a subscription follows; each kind names the parts it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeResourceKind {
    /// The list of areas.
    Areas,
    /// `area`'s Page tree.
    Pages,
    /// `page`'s Views and their order.
    Views,
    /// `block`'s content on its home `page`.
    Block,
}

/// A resource whose readers refresh when it changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BridgeResource {
    /// Its kind.
    pub kind: BridgeResourceKind,
    /// An area.
    pub area: Option<String>,
    /// A Page.
    pub page: Option<String>,
    /// A Block.
    pub block: Option<String>,
}

/// A resource changed; `seq` counts its changes since this Workspace opened, so a reader
/// that sees a gap or an older number knows to query again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeChange {
    /// The resource.
    pub resource: BridgeResource,
    /// Its change count.
    pub seq: i64,
}

type Deliver = Box<dyn FnMut(BridgeChange) -> bool + Send>;

struct Inner {
    workspace: Workspace<Sqlite>,
    /// Open subscriptions by the identity Dart chose.
    subscriptions: HashMap<i64, (BridgeResource, Deliver)>,
    /// Subscriptions whose close arrived before their registration, which calls on
    /// flutter_rust_bridge's worker threads may do.
    closed: HashSet<i64>,
    /// Subscriptions that ended because their stream was gone, until Dart's close arrives.
    ended: HashSet<i64>,
    /// Each changed resource's change count.
    seqs: HashMap<BridgeResource, i64>,
}

/// The device's Workspace, opened from its replica.
#[flutter_rust_bridge::frb(opaque)]
pub struct LiquidWorkspace {
    inner: Mutex<Inner>,
    /// The replica folder's lock, held while the Workspace is open.
    _lock: File,
}

impl Drop for LiquidWorkspace {
    fn drop(&mut self) {
        // A concurrently spawned child may still hold an inherited descriptor until exec.
        let _ = self._lock.unlock();
    }
}

fn message(error: Error) -> String {
    match error {
        Error::NotAllowed => "not allowed".into(),
        Error::NotFound => "not found".into(),
        Error::OutOfRange => "text position out of range".into(),
        Error::NoChange => "the write changes nothing".into(),
        Error::SectionBoundary => "the Text Block starts a Section".into(),
        Error::WrongKind => "the value is not of its field's kind".into(),
        Error::TooMany => "the Database holds the most fields or options it may".into(),
        Error::Random => "the random source failed".into(),
        Error::Document(message) | Error::Storage(message) => message,
    }
}

pub(crate) fn id<T>(
    text: &Option<String>,
    parse: fn(&str) -> Option<T>,
    what: &str,
) -> Result<T, String> {
    text.as_deref()
        .and_then(parse)
        .ok_or_else(|| format!("{what} is missing or no identity"))
}

fn optional_id<T>(
    text: &Option<String>,
    parse: fn(&str) -> Option<T>,
    what: &str,
) -> Result<Option<T>, String> {
    text.as_ref().map(|_| id(text, parse, what)).transpose()
}

impl BridgeMutation {
    /// Where a new field or row goes: last, after `after`, or first.
    fn place<T>(&self, parse: fn(&str) -> Option<T>) -> Result<Place<T>, String> {
        match (self.last, optional_id(&self.after, parse, "after")?) {
            (true, Some(_)) => Err("after and last cannot both be set".into()),
            (true, None) => Ok(Place::Last),
            (false, Some(after)) => Ok(Place::After(after)),
            (false, None) => Ok(Place::First),
        }
    }
}

fn units(value: i64, what: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| format!("{what} is negative"))
}

impl BridgeMutation {
    fn to_core(&self) -> Result<Mutation, String> {
        let text = || {
            self.text
                .clone()
                .ok_or_else(|| "text is missing".to_string())
        };
        let page = || id(&self.page, PageId::parse, "page");
        let block = || id(&self.block, BlockId::parse, "block");
        let field = || id(&self.field, FieldId::parse, "field");
        let row = || id(&self.row, RowId::parse, "row");
        Ok(match self.kind {
            BridgeMutationKind::CreateArea => Mutation::CreateArea { name: text()? },
            BridgeMutationKind::CreatePage => Mutation::CreatePage {
                area: id(&self.area, AreaId::parse, "area")?,
                parent: optional_id(&self.parent, PageId::parse, "parent")?,
                title: text()?,
            },
            BridgeMutationKind::RenamePage => Mutation::RenamePage {
                page: id(&self.page, PageId::parse, "page")?,
                title: text()?,
            },
            BridgeMutationKind::AddTextBlock => Mutation::AddTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                text: text()?,
            },
            BridgeMutationKind::EditText => Mutation::EditText {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                at: units(self.at, "at")?,
                delete: units(self.delete, "delete")?,
                insert: text()?,
            },
            BridgeMutationKind::RemoveView => Mutation::RemoveView {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
            },
            BridgeMutationKind::SplitTextBlock => Mutation::SplitTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                view: id(&self.view, ViewId::parse, "view")?,
                at: units(self.at, "at")?,
                style: self.style.to_core(),
            },
            BridgeMutationKind::JoinTextBlock => Mutation::JoinTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
            },
            BridgeMutationKind::SetTextStyle => Mutation::SetTextStyle {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                style: self.style.to_core(),
            },
            BridgeMutationKind::SetSectionBoundary => Mutation::SetSectionBoundary {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
                boundary: self.boundary,
            },
            BridgeMutationKind::AddDatabaseBlock => Mutation::AddDatabaseBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                name: text()?,
            },
            BridgeMutationKind::RenameDatabase => Mutation::RenameDatabase {
                page: page()?,
                block: block()?,
                name: text()?,
            },
            BridgeMutationKind::AddField => Mutation::AddField {
                page: page()?,
                block: block()?,
                place: self.place(FieldId::parse)?,
                name: text()?,
                kind: self.field_kind.ok_or("field kind is missing")?.to_core(),
            },
            BridgeMutationKind::RenameField => Mutation::RenameField {
                page: page()?,
                block: block()?,
                field: field()?,
                name: text()?,
            },
            BridgeMutationKind::RemoveField => Mutation::RemoveField {
                page: page()?,
                block: block()?,
                field: field()?,
            },
            BridgeMutationKind::AddOption => Mutation::AddOption {
                page: page()?,
                block: block()?,
                field: field()?,
                name: text()?,
                color: u8::try_from(self.color).map_err(|_| "color is out of range")?,
            },
            BridgeMutationKind::AddRow => Mutation::AddRow {
                page: page()?,
                block: block()?,
                place: self.place(RowId::parse)?,
            },
            BridgeMutationKind::RemoveRow => Mutation::RemoveRow {
                page: page()?,
                block: block()?,
                row: row()?,
            },
            BridgeMutationKind::SetCell => Mutation::SetCell {
                page: page()?,
                block: block()?,
                row: row()?,
                field: field()?,
                value: self.cell.as_ref().ok_or("cell is missing")?.to_core()?,
            },
        })
    }
}

impl BridgeResource {
    /// The resource with only the parts its kind uses, which must name identities, so it
    /// equals the resource a change reports.
    fn normalized(&self) -> Result<Self, String> {
        let area = || id(&self.area, AreaId::parse, "area").map(|area| Some(area.to_string()));
        let page = || id(&self.page, PageId::parse, "page").map(|page| Some(page.to_string()));
        let block =
            || id(&self.block, BlockId::parse, "block").map(|block| Some(block.to_string()));
        let (area, page, block) = match self.kind {
            BridgeResourceKind::Areas => (None, None, None),
            BridgeResourceKind::Pages => (area()?, None, None),
            BridgeResourceKind::Views => (None, page()?, None),
            BridgeResourceKind::Block => (None, page()?, block()?),
        };
        Ok(Self {
            kind: self.kind,
            area,
            page,
            block,
        })
    }

    fn of(changed: &Changed) -> Self {
        let (kind, area, page, block) = match changed {
            Changed::Areas => (BridgeResourceKind::Areas, None, None, None),
            Changed::Pages(area) => (
                BridgeResourceKind::Pages,
                Some(area.to_string()),
                None,
                None,
            ),
            Changed::Views(page) => (
                BridgeResourceKind::Views,
                None,
                Some(page.to_string()),
                None,
            ),
            Changed::Block(page, block) => (
                BridgeResourceKind::Block,
                None,
                Some(page.to_string()),
                Some(block.to_string()),
            ),
        };
        Self {
            kind,
            area,
            page,
            block,
        }
    }

    /// The Page whose document this resource reads, if any.
    fn page(&self) -> Option<&str> {
        matches!(
            self.kind,
            BridgeResourceKind::Views | BridgeResourceKind::Block
        )
        .then_some(self.page.as_deref())
        .flatten()
    }
}

/// Opens the replica folder's lock file without truncating it, and refuses one that is a
/// link or no plain file, so opening never writes to a file elsewhere.
fn open_lock(path: &Path) -> Result<File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        rustix::fs::OFlags::NOFOLLOW.bits() as i32,
    );
    // FILE_FLAG_OPEN_REPARSE_POINT: opens a link itself, which the plain-file check refuses.
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(&mut options, 0x0020_0000);
    let lock = options
        .open(path)
        .map_err(|error| format!("the replica lock {}: {error}", path.display()))?;
    let plain = lock
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file();
    if !plain {
        return Err(format!(
            "the replica lock {} is no plain file",
            path.display()
        ));
    }
    Ok(lock)
}

/// How long a held replica lock is tried again: a process this one starts holds the lock
/// it inherited only until it runs its program, while another session holds it for good.
const LOCK_PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Takes the replica's lock, trying again for a moment while it is held.
fn take_lock(lock: &File) -> Result<(), String> {
    let since = std::time::Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if since.elapsed() < LOCK_PATIENCE => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(TryLockError::WouldBlock) => {
                return Err("another Liquid session holds this replica".into());
            }
            Err(TryLockError::Error(error)) => return Err(error.to_string()),
        }
    }
}

/// The window `start..start + limit`, with `limit` capped at [`MAX_WINDOW`].
fn window(start: i64, limit: i64) -> Result<(usize, usize), String> {
    Ok((
        units(start, "start")?,
        units(limit.min(MAX_WINDOW), "limit")?,
    ))
}

impl LiquidWorkspace {
    /// Opens the Workspace whose replica lives in the folder `dir`, creating both on first
    /// use. The replica's key lives in the operating system's keychain (ADR-0237).
    pub fn open(dir: String) -> Result<LiquidWorkspace, String> {
        let dir = Path::new(&dir);
        // Created first, so the folder's canonical path is known when an older key is looked up.
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        Self::open_with(dir, &Keychain)
    }

    /// Opens with `keys` in place of the keychain, for tests.
    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn open_with(dir: &Path, keys: &dyn Keys) -> Result<LiquidWorkspace, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        // Taken before the key is read or made, so two sessions opening a new folder at once
        // cannot each store a key of their own.
        let lock = open_lock(&dir.join("replica.lock"))?;
        take_lock(&lock)?;
        let path = dir.join("replica.db");
        let key = replica_key(dir, keys, path.exists())?;
        let workspace = match Workspace::open(Sqlite::open(&path, &key).map_err(message)?) {
            Ok(workspace) => workspace,
            // A new replica: until accounts exist, the device's person creates and owns it.
            Err(Error::NotFound) => {
                let store = Sqlite::open(&path, &key).map_err(message)?;
                Workspace::create(PersonId::new(random_hex()?), store).map_err(message)?
            }
            Err(error) => return Err(message(error)),
        };
        Ok(Self {
            inner: Mutex::new(Inner {
                workspace,
                subscriptions: HashMap::new(),
                closed: HashSet::new(),
                ended: HashSet::new(),
                seqs: HashMap::new(),
            }),
            _lock: lock,
        })
    }

    /// Applies one write as the device's person through Liquid's interface, then tells the
    /// subscribers of every resource it changed.
    pub fn apply(&self, mutation: BridgeMutation) -> Result<BridgeApplied, String> {
        let mutation = mutation.to_core()?;
        let mutation_page = match &mutation {
            Mutation::CreateArea { .. }
            | Mutation::CreatePage { .. }
            | Mutation::RenamePage { .. } => None,
            Mutation::AddTextBlock { page, .. }
            | Mutation::EditText { page, .. }
            | Mutation::RemoveView { page, .. }
            | Mutation::SplitTextBlock { page, .. }
            | Mutation::JoinTextBlock { page, .. }
            | Mutation::SetTextStyle { page, .. }
            | Mutation::SetSectionBoundary { page, .. }
            | Mutation::AddDatabaseBlock { page, .. }
            | Mutation::RenameDatabase { page, .. }
            | Mutation::AddField { page, .. }
            | Mutation::RenameField { page, .. }
            | Mutation::RemoveField { page, .. }
            | Mutation::AddOption { page, .. }
            | Mutation::AddRow { page, .. }
            | Mutation::RemoveRow { page, .. }
            | Mutation::SetCell { page, .. } => Some(page.clone()),
        };
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let actor = Actor::Person(inner.workspace.owner().clone());
        let result = inner.workspace.apply(Request {
            actor,
            origin: Origin::Ui,
            mutation,
        });
        // A refused write may have loaded its Page too.
        let applied = match result {
            Ok(applied) => applied,
            Err(error) => {
                if let Some(page) = &mutation_page {
                    inner.release_page(page);
                }
                return Err(message(error));
            }
        };
        for changed in &applied.changed {
            inner.notify(BridgeResource::of(changed));
        }
        if let Some(page) = mutation_page.as_ref().or(match &applied.created {
            Some(Created::Page(page)) => Some(page),
            _ => None,
        }) {
            inner.release_page(page);
        }
        let mut answer = BridgeApplied {
            action: applied.action.to_string(),
            area: None,
            page: None,
            block: None,
            view: None,
            field: None,
            row: None,
            option: None,
        };
        match applied.created {
            Some(Created::Area(area)) => answer.area = Some(area.to_string()),
            Some(Created::Page(page)) => answer.page = Some(page.to_string()),
            Some(Created::Block { block, view }) => {
                answer.block = Some(block.to_string());
                answer.view = Some(view.to_string());
            }
            Some(Created::Field(field)) => answer.field = Some(field.to_string()),
            Some(Created::Row(row)) => answer.row = Some(row.to_string()),
            Some(Created::Option(option)) => answer.option = Some(option.to_string()),
            None => {}
        }
        Ok(answer)
    }

    /// Reads one bounded window, one Block's text or one Database's fields.
    pub fn query(&self, query: BridgeQuery) -> Result<BridgeAnswer, String> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let (start, limit) = window(query.start, query.limit)?;
        let mut read = None;
        let result = Self::read(&mut inner.workspace, &query, start, limit, &mut read);
        if let Some(page) = read {
            inner.release_page(&page);
        }
        result
    }

    fn read(
        workspace: &mut Workspace<Sqlite>,
        query: &BridgeQuery,
        start: usize,
        limit: usize,
        read: &mut Option<PageId>,
    ) -> Result<BridgeAnswer, String> {
        let mut answer = BridgeAnswer {
            areas: Vec::new(),
            pages: Vec::new(),
            views: Vec::new(),
            count: 0,
            text: None,
            style: None,
            database: None,
            rows: Vec::new(),
        };
        match query.kind {
            BridgeQueryKind::Areas => {
                let areas = workspace.areas().map_err(message)?;
                answer.count = areas.len() as i64;
                answer.areas = areas
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|area| BridgeArea {
                        id: area.id.to_string(),
                        name: area.name,
                    })
                    .collect();
            }
            BridgeQueryKind::Pages => {
                let pages = workspace
                    .pages(&id(&query.area, AreaId::parse, "area")?)
                    .map_err(message)?;
                answer.count = pages.len() as i64;
                answer.pages = pages
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|page| BridgePage {
                        id: page.id.to_string(),
                        parent: page.parent.map(|parent| parent.to_string()),
                        title: page.title,
                    })
                    .collect();
            }
            BridgeQueryKind::Views => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                answer.count = workspace.view_count(&page).map_err(message)? as i64;
                answer.views = workspace
                    .views(&page, start, limit)
                    .map_err(message)?
                    .into_iter()
                    .map(|view| BridgeView {
                        id: view.id.to_string(),
                        block: view.block.to_string(),
                        boundary: view.boundary,
                        kind: match view.kind {
                            BlockKind::Text => BridgeBlockKind::Text,
                            BlockKind::Database => BridgeBlockKind::Database,
                            BlockKind::Unknown => BridgeBlockKind::Unknown,
                        },
                    })
                    .collect();
            }
            BridgeQueryKind::Text => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.text = workspace.text(&page, &block).map_err(message)?;
                answer.style = workspace
                    .style(&page, &block)
                    .map_err(message)?
                    .map(BridgeTextStyle::of);
            }
            BridgeQueryKind::Database => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                answer.database = workspace
                    .database(&page, &block)
                    .map_err(message)?
                    .map(BridgeDatabase::of);
            }
            BridgeQueryKind::Rows => {
                let page = id(&query.page, PageId::parse, "page")?;
                *read = Some(page.clone());
                let block = id(&query.block, BlockId::parse, "block")?;
                let rows = row_query(&query.sort, &query.filter, start, limit)?;
                let rows = workspace.rows(&page, &block, &rows).map_err(message)?;
                answer.count = rows.count as i64;
                answer.rows = rows.rows.into_iter().map(BridgeRow::of).collect();
            }
        }
        Ok(answer)
    }

    /// Streams `resource`'s changes to Dart under the identity `subscription`, which Dart
    /// chooses and passes to [`LiquidWorkspace::close`] when it cancels the stream. The first
    /// change carries the resource's current count and marks the registration: a reader queries
    /// after it, so it misses no later change.
    pub fn subscribe(
        &self,
        subscription: i64,
        resource: BridgeResource,
        sink: StreamSink<BridgeChange>,
    ) -> Result<(), String> {
        // Dart's stream sees only what the sink carries, so a refusal travels through it.
        let refused = sink.clone();
        let watched = self.watch(subscription, resource, move |change| {
            sink.add(change).is_ok()
        });
        if let Err(error) = watched {
            let _ = refused.add_error(error);
        }
        Ok(())
    }

    /// Hands `resource`'s changes to `deliver` until it returns `false`; tests use it instead
    /// of a Dart stream. `deliver` runs while the Workspace is locked, so it must not call
    /// the Workspace, as a stream sink never does.
    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn watch(
        &self,
        subscription: i64,
        resource: BridgeResource,
        deliver: impl FnMut(BridgeChange) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut deliver = deliver;
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        // A refused registration ends the subscription like any other end.
        let resource = match resource.normalized() {
            Ok(resource) => resource,
            Err(error) => {
                if !inner.closed.remove(&subscription) {
                    inner.ended.insert(subscription);
                }
                return Err(error);
            }
        };
        // Dart cancelled it before it arrived, which is a normal end, not an error.
        if inner.closed.remove(&subscription) {
            return Ok(());
        }
        if inner.subscriptions.contains_key(&subscription) {
            return Err(format!("subscription {subscription} is already open"));
        }
        let seq = inner.seqs.get(&resource).copied().unwrap_or_default();
        let current = BridgeChange {
            resource: resource.clone(),
            seq,
        };
        if deliver(current) {
            inner
                .subscriptions
                .insert(subscription, (resource, Box::new(deliver)));
        } else {
            inner.ended.insert(subscription);
        }
        Ok(())
    }

    /// Ends a subscription, also one whose registration has not arrived yet. A Page that no
    /// subscription follows any more leaves memory.
    pub fn close(&self, subscription: i64) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match inner.subscriptions.remove(&subscription) {
            Some((resource, _)) => inner.release(&resource),
            None if inner.ended.remove(&subscription) => {}
            None => {
                inner.closed.insert(subscription);
            }
        }
    }
}

#[cfg(test)]
impl LiquidWorkspace {
    pub(crate) fn is_loaded(&self, page: &PageId) -> bool {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.workspace.is_loaded(page)
    }

    /// How many subscription identities wait for their other end: a registration or a close.
    pub(crate) fn lifecycle_markers(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.closed.len() + inner.ended.len()
    }
}

impl Inner {
    /// Counts a change of `resource` and tells its subscribers, ending those whose Dart
    /// stream is gone.
    fn notify(&mut self, resource: BridgeResource) {
        let seq = self.seqs.entry(resource.clone()).or_default();
        *seq += 1;
        let change = BridgeChange {
            resource: resource.clone(),
            seq: *seq,
        };
        let mut gone = Vec::new();
        for (id, (followed, deliver)) in &mut self.subscriptions {
            if *followed == resource && !deliver(change.clone()) {
                gone.push(*id);
            }
        }
        for id in gone {
            if let Some((resource, _)) = self.subscriptions.remove(&id) {
                self.ended.insert(id);
                self.release(&resource);
            }
        }
    }

    /// Closes the Page `resource` reads once no other subscription follows it.
    fn release(&mut self, resource: &BridgeResource) {
        if let Some(page) = resource.page().and_then(PageId::parse) {
            self.release_page(&page);
        }
    }

    /// Closes `page` unless a subscription follows it, so a Page that a call loaded for
    /// nobody does not stay in memory.
    fn release_page(&mut self, page: &PageId) {
        let followed = self
            .subscriptions
            .values()
            .any(|(other, _)| other.page() == Some(page.as_str()));
        if !followed {
            self.workspace.close_page(page);
        }
    }
}
