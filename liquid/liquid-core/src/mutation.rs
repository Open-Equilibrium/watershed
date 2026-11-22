use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    ActionId, AreaId, AttachmentId, BlockId, FieldId, Layout, OptionId, PageId, PersonId, RowId,
    TabId, TabSetup, ViewId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    Person(PersonId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Ui,
    Api,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextStyle {
    Paragraph,
    Heading(u8),
    Bullet,
    Numbered,
    Todo { done: bool },
    Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Text,
    Number,
    Select,
    Date,
    Checkbox,
    File,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Cell {
    Empty,
    Text(String),
    Number(f64),
    Select(OptionId),
    Date(String),
    Checkbox(bool),
    Files(Vec<crate::Media>),
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Text(a), Self::Text(b)) | (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Select(a), Self::Select(b)) => a == b,
            (Self::Checkbox(a), Self::Checkbox(b)) => a == b,
            (Self::Files(a), Self::Files(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Cell {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place<T> {
    First,
    After(T),
    Last,
}

fn stored_place<'de, D, T>(deserializer: D) -> Result<Place<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored<T> {
        Place(Place<T>),
        After(Option<T>),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Place(place) => place,
        Stored::After(after) => after.map_or(Place::First, Place::After),
    })
}

impl<T> Place<T> {
    pub(crate) fn as_ref(&self) -> Place<&T> {
        match self {
            Self::First => Place::First,
            Self::After(after) => Place::After(after),
            Self::Last => Place::Last,
        }
    }

    pub(crate) fn map<U>(self, change: impl FnOnce(T) -> U) -> Place<U> {
        match self {
            Self::First => Place::First,
            Self::After(after) => Place::After(change(after)),
            Self::Last => Place::Last,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    CreateArea {
        name: String,
    },
    RenameWorkspace {
        name: String,
    },
    SetWorkspaceIcon {
        icon: String,
    },
    CreatePage {
        area: AreaId,
        parent: Option<PageId>,
        title: String,
    },
    RenamePage {
        page: PageId,
        title: String,
    },
    AddTextBlock {
        page: PageId,
        after: Option<ViewId>,
        text: String,
    },
    EditText {
        page: PageId,
        block: BlockId,
        at: usize,
        delete: usize,
        insert: String,
    },
    RemoveView {
        page: PageId,
        view: ViewId,
    },
    AddTab {
        page: PageId,
        view: ViewId,
        after: Option<TabId>,
        setup: Box<TabSetup>,
    },
    RenameTab {
        page: PageId,
        view: ViewId,
        tab: TabId,
        name: String,
    },
    SetTabLayout {
        page: PageId,
        view: ViewId,
        tab: TabId,
        layout: Layout,
    },
    SetTabGroup {
        page: PageId,
        view: ViewId,
        tab: TabId,
        group: Option<FieldId>,
    },
    SetTabDate {
        page: PageId,
        view: ViewId,
        tab: TabId,
        date: Option<FieldId>,
    },
    #[serde(alias = "SetViewSort")]
    SetTabSort {
        page: PageId,
        view: ViewId,
        #[serde(default)]
        tab: TabId,
        sort: Option<crate::Sort>,
    },
    #[serde(alias = "SetViewFilter")]
    SetTabFilter {
        page: PageId,
        view: ViewId,
        #[serde(default)]
        tab: TabId,
        filter: Option<crate::Filter>,
    },
    MoveTab {
        page: PageId,
        view: ViewId,
        tab: TabId,
        after: Option<TabId>,
    },
    RemoveTab {
        page: PageId,
        view: ViewId,
        tab: TabId,
    },
    SplitTextBlock {
        page: PageId,
        block: BlockId,
        view: ViewId,
        at: usize,
        style: TextStyle,
    },
    JoinTextBlock {
        page: PageId,
        view: ViewId,
    },
    SetTextStyle {
        page: PageId,
        block: BlockId,
        style: TextStyle,
    },
    SetSectionBoundary {
        page: PageId,
        view: ViewId,
        boundary: bool,
    },
    TurnIntoCode {
        page: PageId,
        block: BlockId,
    },
    TurnIntoText {
        page: PageId,
        block: BlockId,
    },
    SetCodeLanguage {
        page: PageId,
        block: BlockId,
        language: String,
    },
    AddMediaBlock {
        page: PageId,
        after: Option<ViewId>,
        name: String,
        media_type: String,
        attachment: AttachmentId,
    },
    AddWhiteboardBlock {
        page: PageId,
        after: Option<ViewId>,
    },
    #[serde(alias = "SetWhiteboardStyle")]
    SetDrawingStyle {
        page: PageId,
        block: BlockId,
        style: crate::DrawingStyle,
    },
    #[serde(alias = "EditWhiteboard")]
    EditDrawing {
        page: PageId,
        block: BlockId,
        add: Vec<crate::Element>,
        change: Vec<crate::ElementChange>,
        remove: Vec<crate::ElementId>,
    },
    AddDatabaseBlock {
        page: PageId,
        after: Option<ViewId>,
        name: String,
        #[serde(default)]
        layout: Layout,
    },
    DuplicateTable {
        page: PageId,
        after: Option<ViewId>,
        table: BlockId,
        name: String,
        tabs: Vec<TabSetup>,
    },
    CreateTable {
        area: AreaId,
        name: String,
    },
    AddTableView {
        page: PageId,
        after: Option<ViewId>,
        table: BlockId,
        title: String,
        #[serde(default)]
        tabs: Vec<TabSetup>,
    },
    RenameView {
        page: PageId,
        view: ViewId,
        title: String,
    },
    DeleteTable {
        block: BlockId,
    },
    RenameDatabase {
        block: BlockId,
        name: String,
    },
    AddField {
        block: BlockId,
        #[serde(alias = "after", deserialize_with = "stored_place")]
        place: Place<FieldId>,
        name: String,
        kind: FieldKind,
    },
    RenameField {
        block: BlockId,
        field: FieldId,
        name: String,
    },
    RemoveField {
        block: BlockId,
        field: FieldId,
    },
    AddOption {
        block: BlockId,
        field: FieldId,
        name: String,
        color: u8,
    },
    AddRow {
        block: BlockId,
        #[serde(alias = "after", deserialize_with = "stored_place")]
        place: Place<RowId>,
    },
    RemoveRow {
        block: BlockId,
        row: RowId,
    },
    SetCell {
        block: BlockId,
        row: RowId,
        field: FieldId,
        value: Cell,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub actor: Actor,
    pub origin: Origin,
    pub mutation: Mutation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Created {
    Area(AreaId),
    Page(PageId),
    Block { block: BlockId, view: ViewId },
    Table(BlockId),
    Field(FieldId),
    Row(RowId),
    Option(OptionId),
    Tab(TabId),
    Elements(Vec<crate::ElementId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changed {
    Workspace,
    Areas,
    Pages(AreaId),
    Views(PageId),
    Block(PageId, BlockId),
    Tables,
    Table(BlockId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub action: ActionId,
    pub created: Option<Created>,
    pub changed: Vec<Changed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotAllowed,
    NotFound,
    OutOfRange,
    NoChange,
    SectionBoundary,
    WrongKind,
    TooMany,
    TooLong,
    TooLarge,
    Changed,
    Random,
    Document(String),
    Storage(String),
}

impl From<automerge::AutomergeError> for Error {
    fn from(error: automerge::AutomergeError) -> Self {
        Self::Document(error.to_string())
    }
}
