//! Liquid's Workspace core: the object model, the one mutation pipeline every write takes,
//! and the History of Actions it records (ADR-0235), over Automerge documents (ADR-0238).
//!
//! The Workspace commits every write to a [`Store`] before acknowledging it;
//! `liquid-storage` implements one on SQLite. The Workspace's data lives in three kinds of
//! Automerge document, each created with its whole top-level structure in its first change:
//!
//! - the Workspace document: its owner and its areas;
//! - one document per area: its Page tree, so the Explorer opens without any Page;
//! - one document per Page: its Blocks and their Views, a Database Block with its fields and
//!   rows.
//!
//! Removing an object marks it deleted instead of deleting it from the document, so a
//! concurrent edit or move cannot bring it back and a revert can restore it unchanged.
//! Every change of every document carries the header of the Action that made it.
//! These are the merge rules of the client architecture's rule 11
//! (`docs/concept/liquid-client-architecture.md`).

#![deny(missing_docs)]

mod attachment;
mod database;
mod drawing;
mod history;
mod id;
mod model;
mod mutation;
mod order;
mod sort_key;
mod store;
mod workspace;

pub use attachment::{Attachment, AttachmentId, MAX_ATTACHMENT, Media, PART, Upload};
pub use database::{
    Database, Field, Filter, MAX_FIELDS, MAX_FILES, MAX_OPTIONS, Match, Row, RowQuery, Rows,
    SelectOption, Sort,
};
pub use drawing::{
    Content, Drawing, DrawingStyle, Element, ElementChange, Frame, InkColor, MAX_BOARD_POINTS,
    MAX_DOC_PAGES, MAX_DRAWING_ELEMENTS, MAX_DRAWING_POINTS, MAX_ELEMENT_TEXT, MAX_ELEMENTS,
    MAX_POINTS, Pen, Shape,
};
pub use history::{Action, Document, Operation, Permission};
pub use id::{
    ActionId, AreaId, BlockId, ElementId, FieldId, OptionId, PageId, PersonId, RowId, ViewId,
};
pub use model::{Area, BlockKind, Code, PageEntry, View};
pub use mutation::{
    Actor, Applied, Cell, Changed, Created, Error, FieldKind, Mutation, Origin, Place, Request,
    TextStyle,
};
pub use store::{Commit, MemoryStore, Store, Write};
pub use workspace::Workspace;

#[cfg(test)]
mod tests;
