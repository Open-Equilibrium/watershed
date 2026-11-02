//! The requests the mutation pipeline accepts and what it answers. These types are the
//! schema that the UI, the CLI/API, agents and sync share (client architecture rule 2).

use serde::{Deserialize, Serialize};

use crate::{ActionId, AreaId, BlockId, PageId, PersonId, ViewId};

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

/// One write to the Workspace. Text positions count UTF-16 code units, as Dart and
/// JavaScript do. A Page-scoped mutation names the Page whose document holds its target:
/// a Block's home Page.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The system's random source failed, so no new identity could be made.
    Random,
    /// The CRDT library failed; the message is its own.
    Document(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAllowed => f.write_str("not allowed"),
            Self::NotFound => f.write_str("not found"),
            Self::OutOfRange => f.write_str("text position out of range"),
            Self::NoChange => f.write_str("the edit changes nothing"),
            Self::Random => f.write_str("the random source failed"),
            Self::Document(message) => write!(f, "document error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<automerge::AutomergeError> for Error {
    fn from(error: automerge::AutomergeError) -> Self {
        Self::Document(error.to_string())
    }
}
