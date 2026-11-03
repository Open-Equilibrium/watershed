//! The requests the mutation pipeline accepts and what it answers. These types are the
//! schema that the UI, the CLI/API, agents and sync share (client architecture rule 2).

use serde::{Deserialize, Serialize};

use crate::{ActionId, AreaId, BlockId, FieldId, OptionId, PageId, PersonId, RowId, ViewId};

/// Who makes a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// A person.
    Person(PersonId),
}

/// The surface a request comes through.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Liquid's own interface.
    Ui,
    /// The Workspace CLI/API.
    Api,
}

/// How a Text Block reads: one paragraph, heading, list item, to-do or quote (ADR-0235).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextStyle {
    /// A paragraph.
    Paragraph,
    /// A heading of level 1 to 3.
    Heading(u8),
    /// A bulleted list item.
    Bullet,
    /// A numbered list item.
    Numbered,
    /// A to-do, done or not.
    Todo {
        /// Whether it is done.
        done: bool,
    },
    /// A quote.
    Quote,
}

/// The kind of a Database field, which decides the values its cells take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    /// Plain text.
    Text,
    /// A finite number.
    Number,
    /// One of the field's options.
    Select,
    /// A day, `YYYY-MM-DD`; times follow once their zones are decided.
    Date,
    /// Ticked or not.
    Checkbox,
}

/// One cell's value. A cell of a field takes only values of the field's kind.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Cell {
    /// Nothing.
    Empty,
    /// Text.
    Text(String),
    /// A finite number.
    Number(f64),
    /// An option of the field.
    Select(OptionId),
    /// A day, `YYYY-MM-DD`.
    Date(String),
    /// Ticked or not.
    Checkbox(bool),
}

// Numbers compare by their bits, so every Cell equals itself, also one holding NaN, which
// the pipeline refuses.
impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Text(a), Self::Text(b)) | (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Select(a), Self::Select(b)) => a == b,
            (Self::Checkbox(a), Self::Checkbox(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Cell {}

/// One write to the Workspace. Text positions count UTF-16 code units, as Dart and
/// JavaScript do. A Page-scoped mutation names the Page whose document holds its target:
/// a Block's home Page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    /// Adds an area after the existing ones.
    CreateArea {
        /// Its name.
        name: String,
    },
    /// Adds a Page after its siblings, at the top of `area` or under `parent`.
    CreatePage {
        /// The area whose tree holds it.
        area: AreaId,
        /// The Page it nests under.
        parent: Option<PageId>,
        /// Its title.
        title: String,
    },
    /// Changes a Page's title.
    RenamePage {
        /// The Page.
        page: PageId,
        /// The new title.
        title: String,
    },
    /// Adds a Text Block with a View on `page`, at its start or after the View `after`.
    AddTextBlock {
        /// The Page.
        page: PageId,
        /// The View it follows.
        after: Option<ViewId>,
        /// Its text.
        text: String,
    },
    /// Replaces `delete` units at `at` in a Text Block with `insert`.
    EditText {
        /// The Block's home Page.
        page: PageId,
        /// The Text Block.
        block: BlockId,
        /// Where the edit starts.
        at: usize,
        /// How many units it removes.
        delete: usize,
        /// What it inserts there.
        insert: String,
    },
    /// Removes a View; removing a Block's last View removes the Block in the same Action.
    RemoveView {
        /// The Page.
        page: PageId,
        /// The View.
        view: ViewId,
    },
    /// Ends a Text Block at `at`, as ⏎ does: its text from there moves to a new Text Block of
    /// `style`, shown right after `view`. At the very start, as in Notion, the text stays in
    /// its Block and the new, empty Text Block goes above it.
    SplitTextBlock {
        /// The Block's home Page, which `view` is on.
        page: PageId,
        /// The Text Block.
        block: BlockId,
        /// Its View the new Text Block follows.
        view: ViewId,
        /// Where the text splits.
        at: usize,
        /// How the new Text Block reads.
        style: TextStyle,
    },
    /// Appends a Text Block to the one above, as ⌫ at its start does, and removes its View;
    /// the Block goes with its last View. A View that starts a Section joins nothing.
    JoinTextBlock {
        /// The Page.
        page: PageId,
        /// The View of the Text Block that joins the one above.
        view: ViewId,
    },
    /// Changes how a Text Block reads.
    SetTextStyle {
        /// The Block's home Page.
        page: PageId,
        /// The Text Block.
        block: BlockId,
        /// Its new style.
        style: TextStyle,
    },
    /// Starts or stops a Section at a View, as `---` and ⌫ below a divider do. A boundary
    /// belongs to the View, so a Text Block shown on two Pages joins each Page's Sections. At
    /// the Page's start or after a Block of another type a boundary already exists, so one set
    /// there changes nothing visible.
    SetSectionBoundary {
        /// The Page.
        page: PageId,
        /// The View.
        view: ViewId,
        /// Whether a Section starts here.
        boundary: bool,
    },
    /// Adds a Database Block with one text field, "Name", and a View on `page`, at its
    /// start or after the View `after`.
    AddDatabaseBlock {
        /// The Page.
        page: PageId,
        /// The View it follows.
        after: Option<ViewId>,
        /// Its name.
        name: String,
    },
    /// Renames a Database Block.
    RenameDatabase {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The new name.
        name: String,
    },
    /// Adds a field first or after the field `after`.
    AddField {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The field it follows.
        after: Option<FieldId>,
        /// Its name.
        name: String,
        /// Its kind.
        kind: FieldKind,
    },
    /// Renames a field.
    RenameField {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The field.
        field: FieldId,
        /// The new name.
        name: String,
    },
    /// Removes a field; its cells stay in the document but read as nothing.
    RemoveField {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The field.
        field: FieldId,
    },
    /// Adds an option to a select field, after its others.
    AddOption {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The select field.
        field: FieldId,
        /// Its name.
        name: String,
        /// Its color, an index into the tag colors that the Table View's mockups define.
        color: u8,
    },
    /// Adds an empty row first or after the row `after`.
    AddRow {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The row it follows.
        after: Option<RowId>,
    },
    /// Removes a row.
    RemoveRow {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The row.
        row: RowId,
    },
    /// Sets one cell; [`Cell::Empty`] clears it.
    SetCell {
        /// The Block's home Page.
        page: PageId,
        /// The Database Block.
        block: BlockId,
        /// The row.
        row: RowId,
        /// The field.
        field: FieldId,
        /// The value, of the field's kind.
        value: Cell,
    },
}

/// A write request: the mutation and who asks for it through which surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Who asks.
    pub actor: Actor,
    /// Through which surface.
    pub origin: Origin,
    /// The write.
    pub mutation: Mutation,
}

/// What an applied mutation created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Created {
    /// A new area.
    Area(AreaId),
    /// A new Page.
    Page(PageId),
    /// A new Block and its first View.
    Block {
        /// The Block.
        block: BlockId,
        /// Its View.
        view: ViewId,
    },
    /// A new Database field.
    Field(FieldId),
    /// A new Database row.
    Row(RowId),
    /// A new option of a select field.
    Option(OptionId),
}

/// A resource whose readers must refresh, so each subscribes only to what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changed {
    /// The list of areas.
    Areas,
    /// An area's Page tree.
    Pages(AreaId),
    /// A Page's Views and their order.
    Views(PageId),
    /// One Block's content.
    Block(PageId, BlockId),
}

/// The answer to an applied request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// The Action that recorded it.
    pub action: ActionId,
    /// What it created.
    pub created: Option<Created>,
    /// What it changed.
    pub changed: Vec<Changed>,
}

/// Why the pipeline refused a request. A refused request changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The actor may not make this change.
    NotAllowed,
    /// A named object does not exist or was removed.
    NotFound,
    /// A text position lies outside the text or inside a character.
    OutOfRange,
    /// The edit would change nothing.
    NoChange,
    /// A Text Block that starts a Section joins no Text Block above it; ⌫ there removes
    /// the boundary instead.
    SectionBoundary,
    /// A cell's value is not of its field's kind, or is no valid value of it.
    WrongKind,
    /// The system's random source failed, so no new identity could be made.
    Random,
    /// The CRDT library failed; the message is its own.
    Document(String),
    /// The store failed, so nothing was acknowledged; the message is the store's own.
    Storage(String),
}

impl From<automerge::AutomergeError> for Error {
    fn from(error: automerge::AutomergeError) -> Self {
        Self::Document(error.to_string())
    }
}
